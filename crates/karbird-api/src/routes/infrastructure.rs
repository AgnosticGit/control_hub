//! Infrastructure view (ADR-0018): machines and containers from the Karbird agent's metrics,
//! joined with service instances from spans, plus the deterministic noisy-neighbour rule.
//! Everything below the handler is a pure function of the query rows.

use std::collections::{BTreeMap, HashMap};

use axum::Json;
use axum::extract::{Path, Query, State};
use karbird_engine_core::{
    ContainerDto, HostDto, InfrastructureSnapshot, InstanceDto, InstanceMatch, NoisyNeighborHint,
};
use karbird_storage::ch::{InfraSeriesRow, InstanceLatencyRow};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::auth::{CurrentUser, authorize};
use crate::error::ApiResult;
use crate::permissions::Permission;
use crate::routes::telemetry::{now_ns, parse_ns};
use crate::state::AppState;

const NS_PER_SEC: i64 = 1_000_000_000;
/// Machine metrics are read from `[at - METRICS_WINDOW, at]` (agents export every 10–60 s).
pub const METRICS_WINDOW_NS: i64 = 120 * NS_PER_SEC;
/// Latency "now" is `[at - 5 min, at)`, the baseline the 30 minutes before.
pub const RECENT_NS: i64 = 300 * NS_PER_SEC;
pub const BASELINE_NS: i64 = 1_800 * NS_PER_SEC;
const MAX_SERIES: u32 = 20_000;
const MAX_INSTANCES: u32 = 2_000;

// Noisy-neighbour rule: all thresholds in one place, documented in ADR-0018.
pub const MIN_SAMPLES: u64 = 20;
pub const SLOWDOWN_RATIO: f64 = 1.5;
pub const SLOWDOWN_MIN_MS: f64 = 20.0;
pub const NEIGHBOR_CPU_SHARE: f64 = 0.8;
pub const HOST_CPU_BUSY: f64 = 0.9;
pub const TOP_CONSUMER_SHARE: f64 = 0.5;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InfrastructureParams {
    /// Moment to show, unix epoch nanoseconds as a decimal string. Default: now.
    pub at: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/infrastructure", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), InfrastructureParams),
    responses((status = 200, body = InfrastructureSnapshot), (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn infrastructure(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<InfrastructureParams>,
) -> ApiResult<Json<InfrastructureSnapshot>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    let at = parse_ns(&q.at, "at")?.unwrap_or_else(now_ns);
    let (series, latency) = tokio::try_join!(
        state.ch.infra_series(&scope, at.saturating_sub(METRICS_WINDOW_NS), at, MAX_SERIES),
        state.ch.instance_latency(
            &scope,
            at.saturating_sub(RECENT_NS + BASELINE_NS),
            at.saturating_sub(RECENT_NS),
            at,
            MAX_INSTANCES
        ),
    )?;
    let truncated = series.len() >= MAX_SERIES as usize || latency.len() >= MAX_INSTANCES as usize;
    Ok(Json(build_snapshot(at, &series, &latency, truncated)))
}

// ---------------------------------------------------------------- metrics → machines

fn attr<'a>(row: &'a InfraSeriesRow, key: &str) -> Option<&'a str> {
    row.attributes.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Per-second rate of a cumulative counter over the window; `None` without two points or after
/// a counter reset.
fn rate(row: &InfraSeriesRow) -> Option<f64> {
    let dt = (row.last_time - row.first_time) as f64 / NS_PER_SEC as f64;
    let dv = row.last_value - row.first_value;
    (dt > 0.0 && dv >= 0.0).then_some(dv / dt)
}

fn delta(row: &InfraSeriesRow) -> Option<f64> {
    let dv = row.last_value - row.first_value;
    (row.last_time > row.first_time && dv >= 0.0).then_some(dv)
}

fn sum_some(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values.fold(None, |acc, v| match (acc, v) {
        (a, None) => a,
        (None, Some(v)) => Some(v),
        (Some(a), Some(v)) => Some(a + v),
    })
}

fn finite(v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite())
}

fn by_name<'a>(rows: &'a [&'a InfraSeriesRow], name: &'a str) -> impl Iterator<Item = &'a InfraSeriesRow> + 'a {
    rows.iter().copied().filter(move |r| r.metric_name == name)
}

fn host_metrics(id: &str, rows: &[&InfraSeriesRow]) -> HostDto {
    let named = |n: &'static str| by_name(rows, n);
    let name = rows.iter().map(|r| r.host_name.as_str()).find(|n| !n.is_empty()).unwrap_or(id).to_owned();

    // CPU: busy share of all CPU time in the window, else the utilization gauge.
    let (mut busy, mut total) = (0.0, 0.0);
    let mut cpus = std::collections::BTreeSet::new();
    for r in named("system.cpu.time") {
        if let Some(cpu) = attr(r, "cpu") {
            cpus.insert(cpu.to_owned());
        }
        if let Some(d) = delta(r) {
            total += d;
            if !matches!(attr(r, "state"), Some("idle" | "wait")) {
                busy += d;
            }
        }
    }
    let mut cpu_utilization = (total > 0.0).then(|| (busy / total).clamp(0.0, 1.0));
    if cpu_utilization.is_none() {
        let idle: Vec<f64> = named("system.cpu.utilization")
            .filter(|r| attr(r, "state") == Some("idle"))
            .map(|r| r.last_value)
            .collect();
        if !idle.is_empty() {
            cpu_utilization = Some((1.0 - idle.iter().sum::<f64>() / idle.len() as f64).clamp(0.0, 1.0));
        }
    }
    let cpu_cores = named("system.cpu.logical.count")
        .map(|r| r.last_value)
        .reduce(f64::max)
        .or_else(|| (!cpus.is_empty()).then_some(cpus.len() as f64));

    let memory_used_bytes =
        sum_some(named("system.memory.usage").filter(|r| attr(r, "state") == Some("used")).map(|r| Some(r.last_value)));
    let memory_total_bytes = named("system.memory.limit")
        .map(|r| r.last_value)
        .reduce(f64::max)
        .or_else(|| sum_some(named("system.memory.usage").map(|r| Some(r.last_value))));

    // Disk: the root filesystem, else the largest one.
    let mut fs: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for r in named("system.filesystem.usage") {
        let mount = attr(r, "mountpoint").unwrap_or("");
        let e = fs.entry(mount).or_default();
        e.1 += r.last_value;
        if attr(r, "state") == Some("used") {
            e.0 += r.last_value;
        }
    }
    let disk = fs.get("/").copied().or_else(|| fs.values().copied().max_by(|a, b| a.1.total_cmp(&b.1)));

    let net = |direction: &str| {
        sum_some(
            named("system.network.io")
                .filter(|r| attr(r, "device") != Some("lo") && attr(r, "direction") == Some(direction))
                .map(rate),
        )
    };

    HostDto {
        id: id.to_owned(),
        name,
        has_metrics: true,
        cpu_utilization: finite(cpu_utilization),
        cpu_cores: finite(cpu_cores),
        memory_used_bytes: finite(memory_used_bytes),
        memory_total_bytes: finite(memory_total_bytes),
        disk_used_bytes: finite(disk.map(|d| d.0)),
        disk_total_bytes: finite(disk.map(|d| d.1)),
        network_rx_bytes_per_sec: finite(net("receive")),
        network_tx_bytes_per_sec: finite(net("transmit")),
        containers: Vec::new(),
        last_seen_ns: rows.iter().map(|r| r.last_time).max(),
    }
}

fn container_metrics(id: &str, rows: &[&InfraSeriesRow], host: &HostDto) -> ContainerDto {
    let named = |n: &'static str| by_name(rows, n);
    let name =
        rows.iter().map(|r| r.container_name.trim_start_matches('/')).find(|n| !n.is_empty()).unwrap_or(id).to_owned();
    // Nanoseconds of CPU per second = cores in use; docker_stats' utilization is % of one core.
    let cpu_cores = sum_some(named("container.cpu.usage.total").map(|r| rate(r).map(|ns| ns / NS_PER_SEC as f64)))
        .or_else(|| named("container.cpu.utilization").map(|r| r.last_value / 100.0).reduce(f64::max));
    let cpu_share = match (cpu_cores, host.cpu_cores) {
        (Some(c), Some(h)) if h > 0.0 => Some((c / h).clamp(0.0, 1.0)),
        _ => None,
    };
    let memory_used_bytes = named("container.memory.usage.total").map(|r| r.last_value).reduce(f64::max);
    let memory_limit_bytes = named("container.memory.usage.limit").map(|r| r.last_value).reduce(f64::max);
    let memory_share = match (memory_used_bytes, host.memory_total_bytes.or(memory_limit_bytes)) {
        (Some(u), Some(t)) if t > 0.0 => Some((u / t).clamp(0.0, 1.0)),
        _ => None,
    };
    ContainerDto {
        id: id.to_owned(),
        name,
        service_id: None,
        service_name: rows.iter().map(|r| r.service_name.clone()).find(|n| !n.is_empty()),
        cpu_cores: finite(cpu_cores),
        cpu_share: finite(cpu_share),
        memory_used_bytes: finite(memory_used_bytes),
        memory_limit_bytes: finite(memory_limit_bytes),
        memory_share: finite(memory_share),
        network_rx_bytes_per_sec: finite(sum_some(named("container.network.io.usage.rx_bytes").map(rate))),
        network_tx_bytes_per_sec: finite(sum_some(named("container.network.io.usage.tx_bytes").map(rate))),
    }
}

pub fn hosts_from_series(series: &[InfraSeriesRow]) -> Vec<HostDto> {
    let mut by_host: BTreeMap<&str, Vec<&InfraSeriesRow>> = BTreeMap::new();
    for r in series.iter().filter(|r| !r.host_id.is_empty()) {
        by_host.entry(r.host_id.as_str()).or_default().push(r);
    }
    let mut hosts = Vec::new();
    for (id, rows) in by_host {
        let machine: Vec<&InfraSeriesRow> = rows.iter().copied().filter(|r| r.container_id.is_empty()).collect();
        let mut host = host_metrics(id, &machine);
        host.last_seen_ns = rows.iter().map(|r| r.last_time).max();
        if host.name == id
            && let Some(n) = rows.iter().map(|r| r.host_name.as_str()).find(|n| !n.is_empty())
        {
            host.name = n.to_owned();
        }
        let mut by_container: BTreeMap<&str, Vec<&InfraSeriesRow>> = BTreeMap::new();
        for r in rows.iter().copied().filter(|r| !r.container_id.is_empty()) {
            by_container.entry(r.container_id.as_str()).or_default().push(r);
        }
        host.containers = by_container.iter().map(|(cid, rs)| container_metrics(cid, rs, &host)).collect();
        hosts.push(host);
    }
    hosts
}

// ---------------------------------------------------------------- instances → machines

fn is_short_id(s: &str) -> bool {
    s.len() >= 12 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Same container: equal IDs, or one is the other's short (≥ 12 hex) prefix.
fn same_container(a: &str, b: &str) -> bool {
    !a.is_empty()
        && !b.is_empty()
        && (a == b || (is_short_id(a) && b.starts_with(a)) || (is_short_id(b) && a.starts_with(b)))
}

struct Matched {
    host_id: Option<String>,
    container_id: Option<String>,
    reason: InstanceMatch,
}

fn match_instance(
    row: &InstanceLatencyRow,
    hosts: &[HostDto],
    hostnames: &HashMap<String, (String, String)>,
) -> Matched {
    let found = |host: &HostDto, c: &ContainerDto, reason| Matched {
        host_id: Some(host.id.clone()),
        container_id: Some(c.id.clone()),
        reason,
    };
    for h in hosts {
        if let Some(c) = h.containers.iter().find(|c| same_container(&c.id, &row.container_id)) {
            return found(h, c, InstanceMatch::ContainerId);
        }
    }
    if !row.host_name.is_empty() {
        if let Some((host, container)) = hostnames.get(&row.host_name) {
            return Matched {
                host_id: Some(host.clone()),
                container_id: Some(container.clone()),
                reason: InstanceMatch::ContainerHostname,
            };
        }
        if is_short_id(&row.host_name) {
            for h in hosts {
                if let Some(c) = h.containers.iter().find(|c| same_container(&c.id, &row.host_name)) {
                    return found(h, c, InstanceMatch::ContainerHostname);
                }
            }
        }
    }
    for key in [&row.host_id, &row.host_name] {
        if let Some(h) = hosts.iter().find(|h| h.has_metrics && !key.is_empty() && (h.id == *key || h.name == *key)) {
            return Matched { host_id: Some(h.id.clone()), container_id: None, reason: InstanceMatch::HostName };
        }
    }
    if !row.host_name.is_empty() {
        return Matched { host_id: Some(row.host_name.clone()), container_id: None, reason: InstanceMatch::Unverified };
    }
    Matched { host_id: None, container_id: None, reason: InstanceMatch::Unknown }
}

fn ms(ns: f64) -> Option<f64> {
    (ns.is_finite() && ns >= 0.0).then(|| ns / 1e6)
}

pub fn build_snapshot(
    at: i64,
    series: &[InfraSeriesRow],
    latency: &[InstanceLatencyRow],
    truncated: bool,
) -> InfrastructureSnapshot {
    let mut hosts = hosts_from_series(series);
    let hostnames: HashMap<String, (String, String)> = series
        .iter()
        .filter(|r| !r.container_hostname.is_empty() && !r.container_id.is_empty())
        .map(|r| (r.container_hostname.clone(), (r.host_id.clone(), r.container_id.clone())))
        .collect();

    let mut instances = Vec::new();
    for row in latency {
        let m = match_instance(row, &hosts, &hostnames);
        if m.reason == InstanceMatch::Unverified
            && let Some(id) = &m.host_id
            && !hosts.iter().any(|h| &h.id == id)
        {
            hosts.push(HostDto { id: id.clone(), name: id.clone(), has_metrics: false, ..HostDto::default() });
        }
        if let (Some(hid), Some(cid)) = (&m.host_id, &m.container_id)
            && let Some(c) =
                hosts.iter_mut().filter(|h| &h.id == hid).flat_map(|h| h.containers.iter_mut()).find(|c| &c.id == cid)
        {
            c.service_id = Some(row.service_id.clone());
            c.service_name = Some(row.service_name.clone());
        }
        instances.push((row, m));
    }

    let mut hints = Vec::new();
    for (row, m) in &instances {
        if let Some(h) = noisy_neighbor(row, m, &hosts) {
            hints.push(h);
        }
    }
    hints.sort_by(|a, b| (&a.host_id, &a.service_id, &a.instance_id).cmp(&(&b.host_id, &b.service_id, &b.instance_id)));

    let mut notes = Vec::new();
    if !hosts.iter().any(|h| h.has_metrics) {
        notes.push(
            "No machine metrics in this window: run the Karbird agent on each machine to see CPU, memory and containers."
                .to_owned(),
        );
    }
    let unplaced = instances.iter().filter(|(_, m)| m.host_id.is_none()).count();
    if unplaced > 0 {
        notes.push(format!(
            "{unplaced} instance(s) report neither host.name nor container.id and are not placed on a machine."
        ));
    }
    let unverified = instances.iter().filter(|(_, m)| m.reason == InstanceMatch::Unverified).count();
    if unverified > 0 {
        notes.push(format!(
            "{unverified} instance(s) are placed only by their own host.name; no agent confirms that machine."
        ));
    }
    if truncated {
        notes.push("Too many series or instances; the view is incomplete.".to_owned());
    }

    for h in &mut hosts {
        h.containers.sort_by(|a, b| {
            b.cpu_share.unwrap_or(-1.0).total_cmp(&a.cpu_share.unwrap_or(-1.0)).then_with(|| a.name.cmp(&b.name))
        });
    }
    hosts.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));

    InfrastructureSnapshot {
        at_ns: at,
        window_ns: METRICS_WINDOW_NS,
        hosts,
        instances: instances
            .into_iter()
            .map(|(row, m)| InstanceDto {
                service_id: row.service_id.clone(),
                service_name: row.service_name.clone(),
                instance_id: if row.service_instance_id.is_empty() {
                    format!("{}@{}", row.service_name, m.host_id.as_deref().unwrap_or("unknown"))
                } else {
                    row.service_instance_id.clone()
                },
                host_id: m.host_id,
                container_id: m.container_id,
                match_reason: m.reason,
                p95_ms: (row.requests > 0).then(|| ms(row.p95_ns)).flatten(),
                baseline_p95_ms: (row.baseline_requests > 0).then(|| ms(row.baseline_p95_ns)).flatten(),
                requests: u32::try_from(row.requests).unwrap_or(u32::MAX),
            })
            .collect(),
        hints,
        notes,
    }
}

// ---------------------------------------------------------------- noisy neighbour

/// The rule (ADR-0018), evaluated per instance:
/// 1. p95 of the last 5 min ≥ 1.5 × the 30 min before it and ≥ 20 ms more, both from ≥ 20 requests;
/// 2. on the same machine (with agent metrics) another container uses ≥ 80 % of the machine's CPU,
///    or the machine is ≥ 90 % busy and that container is the top consumer with ≥ 50 % of it.
fn noisy_neighbor(row: &InstanceLatencyRow, m: &Matched, hosts: &[HostDto]) -> Option<NoisyNeighborHint> {
    if row.requests < MIN_SAMPLES || row.baseline_requests < MIN_SAMPLES {
        return None;
    }
    let (now, base) = (ms(row.p95_ns)?, ms(row.baseline_p95_ns)?);
    if now < base * SLOWDOWN_RATIO || now - base < SLOWDOWN_MIN_MS {
        return None;
    }
    let host = hosts.iter().find(|h| h.has_metrics && Some(&h.id) == m.host_id.as_ref())?;
    let top = host
        .containers
        .iter()
        .filter(|c| Some(&c.id) != m.container_id.as_ref() && c.service_id.as_deref() != Some(row.service_id.as_str()))
        .filter_map(|c| c.cpu_share.map(|s| (s, c)))
        .max_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.name.cmp(&a.1.name)))?;
    let (share, neighbor) = top;
    let busy_host = host.cpu_utilization.is_some_and(|u| u >= HOST_CPU_BUSY) && share >= TOP_CONSUMER_SHARE;
    if share < NEIGHBOR_CPU_SHARE && !busy_host {
        return None;
    }
    let neighbor_name = neighbor.service_name.clone().unwrap_or_else(|| neighbor.name.clone());
    let pct = |v: f64| (v * 100.0).round() as i64;
    let host_part = host.cpu_utilization.map(|u| format!(", machine at {}%", pct(u))).unwrap_or_default();
    Some(NoisyNeighborHint {
        service_id: row.service_id.clone(),
        service_name: row.service_name.clone(),
        instance_id: if row.service_instance_id.is_empty() {
            format!("{}@{}", row.service_name, host.id)
        } else {
            row.service_instance_id.clone()
        },
        neighbor_service_id: neighbor.service_id.clone(),
        neighbor_name: neighbor_name.clone(),
        neighbor_container_id: neighbor.id.clone(),
        host_id: host.id.clone(),
        host_name: host.name.clone(),
        p95_ms: now,
        baseline_p95_ms: base,
        neighbor_cpu_share: share,
        host_cpu_utilization: host.cpu_utilization,
        reason: format!(
            "{} p95 {:.0} ms vs {:.0} ms before (×{:.1}); {} uses {}% of {} CPU{}",
            row.service_name,
            now,
            base,
            now / base.max(f64::MIN_POSITIVE),
            neighbor_name,
            pct(share),
            host.name,
            host_part
        ),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    const T: i64 = 1_790_000_000 * NS_PER_SEC;
    const MB: f64 = 1_048_576.0;

    fn series(
        host: &str,
        container: &str,
        metric: &str,
        attrs: &[(&str, &str)],
        first: f64,
        last: f64,
    ) -> InfraSeriesRow {
        InfraSeriesRow {
            host_id: host.into(),
            host_name: host.into(),
            container_id: container.into(),
            container_name: if container.is_empty() { String::new() } else { format!("/{container}-name") },
            container_hostname: String::new(),
            service_name: String::new(),
            metric_name: metric.into(),
            series_id: 0,
            attributes: attrs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
            first_value: first,
            first_time: T - 60 * NS_PER_SEC,
            last_value: last,
            last_time: T,
        }
    }

    /// vps-app: 1 core, 92 % busy, 870/1024 MB; inventory burns 0.95 cores, orders 0.02.
    fn vps_app() -> Vec<InfraSeriesRow> {
        vec![
            series("vps-app", "", "system.cpu.time", &[("cpu", "cpu0"), ("state", "user")], 0.0, 50.0),
            series("vps-app", "", "system.cpu.time", &[("cpu", "cpu0"), ("state", "system")], 0.0, 5.2),
            series("vps-app", "", "system.cpu.time", &[("cpu", "cpu0"), ("state", "idle")], 0.0, 4.8),
            series("vps-app", "", "system.memory.usage", &[("state", "used")], 0.0, 870.0 * MB),
            series("vps-app", "", "system.memory.usage", &[("state", "free")], 0.0, 154.0 * MB),
            series("vps-app", "", "system.filesystem.usage", &[("mountpoint", "/"), ("state", "used")], 0.0, 20e9),
            series("vps-app", "", "system.filesystem.usage", &[("mountpoint", "/"), ("state", "free")], 0.0, 30e9),
            series("vps-app", "", "system.filesystem.usage", &[("mountpoint", "/boot"), ("state", "used")], 0.0, 1e9),
            series("vps-app", "", "system.network.io", &[("device", "eth0"), ("direction", "receive")], 0.0, 6e6),
            series("vps-app", "", "system.network.io", &[("device", "lo"), ("direction", "receive")], 0.0, 9e9),
            series("vps-app", "", "system.network.io", &[("device", "eth0"), ("direction", "transmit")], 0.0, 3e6),
            series("vps-app", "1aa123456789abc", "container.cpu.usage.total", &[], 0.0, 57e9),
            series("vps-app", "1aa123456789abc", "container.memory.usage.total", &[], 0.0, 300.0 * MB),
            series("vps-app", "0dd123456789abc", "container.cpu.usage.total", &[], 0.0, 1.2e9),
            series("vps-app", "0dd123456789abc", "container.memory.usage.total", &[], 0.0, 100.0 * MB),
            series(
                "vps-app",
                "0dd123456789abc",
                "container.network.io.usage.rx_bytes",
                &[("interface", "eth0")],
                0.0,
                600.0,
            ),
        ]
    }

    fn latency(service: &str, instance: &str, container: &str, host: &str, p95: f64, base: f64) -> InstanceLatencyRow {
        InstanceLatencyRow {
            service_id: format!("svc_{service}"),
            service_name: service.into(),
            service_instance_id: instance.into(),
            host_name: host.into(),
            host_id: String::new(),
            container_id: container.into(),
            requests: 100,
            p95_ns: p95 * 1e6,
            baseline_requests: 400,
            baseline_p95_ns: base * 1e6,
            last_seen: T,
        }
    }

    fn demo() -> InfrastructureSnapshot {
        build_snapshot(
            T,
            &vps_app(),
            &[
                latency("orders", "orders-1", "0dd123456789abc", "", 410.0, 80.0),
                latency("inventory", "inv-1", "1aa123456789abc", "", 30.0, 28.0),
            ],
            false,
        )
    }

    #[test]
    fn machine_card_numbers_come_from_hostmetrics() {
        let s = demo();
        let h = &s.hosts[0];
        assert_eq!(h.name, "vps-app");
        assert!(h.has_metrics);
        assert!((h.cpu_utilization.unwrap() - 0.92).abs() < 1e-9, "{:?}", h.cpu_utilization);
        assert_eq!(h.cpu_cores, Some(1.0));
        assert_eq!(h.memory_used_bytes, Some(870.0 * MB));
        assert_eq!(h.memory_total_bytes, Some(1024.0 * MB));
        assert_eq!((h.disk_used_bytes, h.disk_total_bytes), (Some(20e9), Some(50e9)), "root filesystem only");
        assert_eq!(h.network_rx_bytes_per_sec, Some(100_000.0), "loopback excluded");
        assert_eq!(h.network_tx_bytes_per_sec, Some(50_000.0));
        assert_eq!(h.last_seen_ns, Some(T));
    }

    #[test]
    fn containers_get_their_share_of_the_machine_and_their_service() {
        let s = demo();
        let c = &s.hosts[0].containers;
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].id, "1aa123456789abc", "sorted by CPU share");
        assert_eq!(c[0].name, "1aa123456789abc-name", "docker's leading slash is dropped");
        assert!((c[0].cpu_cores.unwrap() - 0.95).abs() < 1e-9);
        assert!((c[0].cpu_share.unwrap() - 0.95).abs() < 1e-9);
        assert!((c[0].memory_share.unwrap() - 300.0 / 1024.0).abs() < 1e-9);
        assert_eq!(c[0].service_id.as_deref(), Some("svc_inventory"));
        assert_eq!(c[1].service_name.as_deref(), Some("orders"));
        assert_eq!(c[1].network_rx_bytes_per_sec, Some(10.0));
    }

    #[test]
    fn orders_slowdown_with_inventory_burning_cpu_is_a_noisy_neighbour() {
        let s = demo();
        assert_eq!(s.hints.len(), 1, "{:?}", s.hints);
        let h = &s.hints[0];
        assert_eq!((h.service_name.as_str(), h.neighbor_name.as_str()), ("orders", "inventory"));
        assert_eq!(h.neighbor_service_id.as_deref(), Some("svc_inventory"));
        assert_eq!((h.host_id.as_str(), h.instance_id.as_str()), ("vps-app", "orders-1"));
        assert_eq!((h.p95_ms, h.baseline_p95_ms), (410.0, 80.0));
        assert!(h.reason.contains("inventory uses 95% of vps-app CPU"), "{}", h.reason);
        assert!(h.reason.contains("×5.1"), "{}", h.reason);
        assert!(s.notes.is_empty(), "{:?}", s.notes);
    }

    #[test]
    fn no_hint_without_a_real_slowdown() {
        for (p95, base, why) in
            [(100.0, 80.0, "below 1.5x"), (30.0, 15.0, "2x but only +15 ms"), (f64::NAN, 80.0, "no recent p95")]
        {
            let s = build_snapshot(T, &vps_app(), &[latency("orders", "o", "0dd123456789abc", "", p95, base)], false);
            assert!(s.hints.is_empty(), "{why}");
        }
        let mut few = latency("orders", "o", "0dd123456789abc", "", 400.0, 80.0);
        few.requests = MIN_SAMPLES - 1;
        assert!(build_snapshot(T, &vps_app(), &[few], false).hints.is_empty(), "too few samples");
    }

    #[test]
    fn no_hint_when_no_neighbour_is_busy_enough() {
        let mut rows = vps_app();
        for r in &mut rows {
            if r.container_id == "1aa123456789abc" && r.metric_name == "container.cpu.usage.total" {
                r.last_value = 30e9; // 0.5 cores on a machine that is not saturated
            }
            if r.metric_name == "system.cpu.time" && attr(r, "state") == Some("idle") {
                r.last_value = 40.0;
            }
        }
        let s = build_snapshot(T, &rows, &[latency("orders", "o", "0dd123456789abc", "", 400.0, 80.0)], false);
        assert!(s.hints.is_empty(), "{:?}", s.hints);
    }

    #[test]
    fn a_busy_machine_with_a_dominant_consumer_is_enough() {
        let mut rows = vps_app();
        for r in &mut rows {
            if r.container_id == "1aa123456789abc" && r.metric_name == "container.cpu.usage.total" {
                r.last_value = 36e9; // 0.6 cores: below 80 % but top consumer of a 92 % busy machine
            }
        }
        let s = build_snapshot(T, &rows, &[latency("orders", "o", "0dd123456789abc", "", 400.0, 80.0)], false);
        assert_eq!(s.hints.len(), 1);
        assert_eq!(s.hints[0].host_cpu_utilization.map(|u| (u * 100.0).round()), Some(92.0));
    }

    #[test]
    fn the_victims_own_container_is_never_its_neighbour() {
        let s = build_snapshot(T, &vps_app(), &[latency("inventory", "i", "1aa123456789abc", "", 400.0, 80.0)], false);
        assert!(s.hints.is_empty(), "inventory cannot be its own noisy neighbour: {:?}", s.hints);
    }

    #[test]
    fn instances_match_by_container_id_hostname_or_host() {
        let mut rows = vps_app();
        rows[11].container_hostname = "1aa123456789".into();
        let s = build_snapshot(
            T,
            &rows,
            &[
                latency("a", "a1", "0dd123456789abcdef0000", "", 1.0, 1.0),
                latency("b", "b1", "", "1aa123456789", 1.0, 1.0),
                latency("c", "c1", "", "0dd123456789", 1.0, 1.0),
                latency("d", "d1", "", "vps-app", 1.0, 1.0),
                latency("e", "e1", "", "edge-1", 1.0, 1.0),
                latency("f", "", "", "", 1.0, 1.0),
            ],
            false,
        );
        let by = |svc: &str| s.instances.iter().find(|i| i.service_name == svc).unwrap();
        assert_eq!(by("a").match_reason, InstanceMatch::ContainerId, "full ID vs short ID");
        assert_eq!(by("a").container_id.as_deref(), Some("0dd123456789abc"));
        assert_eq!(by("b").match_reason, InstanceMatch::ContainerHostname);
        assert_eq!(by("b").container_id.as_deref(), Some("1aa123456789abc"));
        assert_eq!(by("c").match_reason, InstanceMatch::ContainerHostname, "short ID as hostname");
        assert_eq!((by("d").match_reason, by("d").host_id.as_deref()), (InstanceMatch::HostName, Some("vps-app")));
        assert_eq!((by("e").match_reason, by("e").host_id.as_deref()), (InstanceMatch::Unverified, Some("edge-1")));
        assert_eq!((by("f").match_reason, by("f").host_id.as_deref()), (InstanceMatch::Unknown, None));
        assert_eq!(by("f").instance_id, "f@unknown");
        let edge = s.hosts.iter().find(|h| h.id == "edge-1").unwrap();
        assert!(!edge.has_metrics && edge.cpu_utilization.is_none());
        assert_eq!(s.notes.len(), 2, "{:?}", s.notes);
    }

    #[test]
    fn without_agent_metrics_the_gap_is_explained() {
        let s = build_snapshot(T, &[], &[latency("orders", "o", "", "vps-app", 1.0, 1.0)], true);
        assert!(s.notes.iter().any(|n| n.contains("Karbird agent")));
        assert!(s.notes.iter().any(|n| n.contains("incomplete")));
        assert_eq!(s.hosts.len(), 1);
        assert!(!s.hosts[0].has_metrics);
        assert!(s.hints.is_empty(), "no rule without machine metrics");
    }

    #[test]
    fn counter_resets_and_single_points_give_no_rate() {
        let mut r = series("h", "", "system.network.io", &[], 100.0, 50.0);
        assert_eq!(rate(&r), None, "reset");
        r.last_value = 100.0;
        r.last_time = r.first_time;
        assert_eq!(rate(&r), None, "one point");
        assert!(same_container("abcdef012345", "abcdef0123456789"));
        assert!(!same_container("abc", "abcdef"), "short prefixes are not IDs");
        assert!(!same_container("", ""));
    }

    #[test]
    fn utilization_gauge_is_the_fallback_for_cpu() {
        let rows = vec![
            series("h", "", "system.cpu.utilization", &[("cpu", "cpu0"), ("state", "idle")], 0.0, 0.2),
            series("h", "", "system.cpu.utilization", &[("cpu", "cpu1"), ("state", "idle")], 0.0, 0.4),
            series("h", "", "system.cpu.logical.count", &[], 2.0, 2.0),
        ];
        let h = &hosts_from_series(&rows)[0];
        assert!((h.cpu_utilization.unwrap() - 0.7).abs() < 1e-9);
        assert_eq!(h.cpu_cores, Some(2.0));
    }

    proptest! {
        #[test]
        fn shares_stay_in_range_and_output_is_deterministic(
            cpu in prop::collection::vec((0.0f64..1e12, 0.0f64..1e12), 1..6),
            mem in 0.0f64..1e12,
        ) {
            let mut rows = vec![series("h", "", "system.cpu.time", &[("cpu", "cpu0"), ("state", "user")], 0.0, 10.0)];
            for (i, (a, b)) in cpu.iter().enumerate() {
                let id = format!("{i:0>12}");
                rows.push(series("h", &id, "container.cpu.usage.total", &[], *a, *b));
                rows.push(series("h", &id, "container.memory.usage.total", &[], 0.0, mem));
            }
            let a = build_snapshot(T, &rows, &[], false);
            let mut rev = rows.clone();
            rev.reverse();
            prop_assert_eq!(&a, &build_snapshot(T, &rev, &[], false));
            for c in &a.hosts[0].containers {
                for v in [c.cpu_share, c.memory_share].into_iter().flatten() {
                    prop_assert!((0.0..=1.0).contains(&v));
                }
            }
        }
    }
}
