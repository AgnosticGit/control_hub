//! Infrastructure timeline (ADR-0018): machine CPU and memory per time bucket, and container
//! lifecycle events derived deterministically from the agent's metrics, so the agent needs no
//! extra support. Everything below the handler is a pure function of the query rows.

use std::collections::{BTreeMap, HashMap};

use axum::Json;
use axum::extract::{Path, Query, State};
use karbird_storage::ch::InfraBucketRow;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{CurrentUser, authorize};
use crate::error::ApiResult;
use crate::permissions::Permission;
use crate::routes::telemetry::{Range, RangeParams};
use crate::state::AppState;

const NS_PER_SEC: i64 = 1_000_000_000;
/// Read before `from`, so the first buckets have a previous value and containers that were
/// already running are not reported as started.
pub const LOOKBACK_NS: i64 = 180 * NS_PER_SEC;
/// A container missing this long while its machine still reports is gone (agents export every
/// 10–60 s); one that appears this long after its machine did has started.
pub const STALE_NS: i64 = 90 * NS_PER_SEC;
/// A container stops and one with the same name starts on the same machine within this time:
/// one redeploy.
pub const REDEPLOY_NS: i64 = 300 * NS_PER_SEC;
/// Memory at or above this share of the limit right before a stop or restart: probably out of memory.
pub const OOM_SHARE: f64 = 0.95;
const DEFAULT_BUCKETS: u32 = 200;
const MAX_BUCKETS: u32 = 1_000;
const MAX_ROWS: u32 = 200_000;
pub const MAX_EVENTS: usize = 500;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InfraTimelineParams {
    /// Start, unix epoch nanoseconds as a decimal string. Default: `to - 15m`.
    pub from: Option<String>,
    /// End (exclusive), unix epoch nanoseconds as a decimal string. Default: now.
    pub to: Option<String>,
    /// Number of buckets (default 200, max 1000).
    pub buckets: Option<u32>,
}

/// Machines over time and what happened to their containers.
#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct InfraTimeline {
    pub from_ns: String,
    pub to_ns: String,
    pub buckets: u32,
    /// Bucket `i` covers `[from + i * bucket_ns, from + (i + 1) * bucket_ns)`.
    pub bucket_ns: String,
    pub hosts: Vec<HostTrack>,
    /// Oldest first, at most 500.
    pub events: Vec<ContainerEvent>,
    pub events_total: u32,
    /// The range holds more metric rows than were read: later buckets may be missing.
    pub truncated: bool,
}

#[derive(Debug, Serialize, ToSchema, PartialEq)]
pub struct HostTrack {
    pub id: String,
    pub name: String,
    /// Busy share of the machine's CPU time per bucket (0–1); `null` without data.
    pub cpu: Vec<Option<f32>>,
    /// Used share of the machine's memory per bucket (0–1); `null` without data.
    pub memory: Vec<Option<f32>>,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerEventKind {
    /// Appeared while its machine was already reporting.
    Started,
    /// Stopped reporting while its machine went on.
    Stopped,
    /// Same container, its CPU counter started over.
    Restarted,
    /// Replaced by a new container with the same name on the same machine.
    Redeployed,
    /// Stopped or restarted with memory at its limit.
    OutOfMemory,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq)]
pub struct ContainerEvent {
    pub time_ns: String,
    pub kind: ContainerEventKind,
    pub host_id: String,
    pub host_name: String,
    pub container_id: String,
    pub container_name: String,
    pub service_name: Option<String>,
    pub detail: String,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/infrastructure/timeline", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), InfraTimelineParams),
    responses((status = 200, body = InfraTimeline), (status = 400, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody)))]
pub async fn infra_timeline(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<InfraTimelineParams>,
) -> ApiResult<Json<InfraTimeline>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    let r = RangeParams { from: q.from.clone(), to: q.to.clone() }.resolve()?;
    let g = Geometry::new(r, q.buckets);
    let rows = state.ch.infra_buckets(&scope, g.base(), r.to, g.bucket_ns, MAX_ROWS).await?;
    let truncated = rows.len() >= MAX_ROWS as usize;
    Ok(Json(build(&g, &rows, truncated)))
}

/// Buckets of the requested range plus the look-back buckets before it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub from: i64,
    pub to: i64,
    pub buckets: u32,
    pub bucket_ns: i64,
    /// Buckets read before `from`: row bucket `k` is output bucket `k - lookback`.
    pub lookback: i64,
}

impl Geometry {
    pub fn new(r: Range, buckets: Option<u32>) -> Self {
        let buckets = buckets.unwrap_or(DEFAULT_BUCKETS).clamp(1, MAX_BUCKETS);
        let span = r.to.saturating_sub(r.from).max(1);
        let bucket_ns = (span + i64::from(buckets) - 1) / i64::from(buckets);
        let lookback = (LOOKBACK_NS + bucket_ns - 1) / bucket_ns;
        Self { from: r.from, to: r.to, buckets, bucket_ns, lookback }
    }

    pub fn base(&self) -> i64 {
        self.from.saturating_sub(self.lookback.saturating_mul(self.bucket_ns))
    }
}

type Series<'a> = BTreeMap<i64, &'a InfraBucketRow>;

fn series<'a>(rows: &[&'a InfraBucketRow], metric: &str, class: &str) -> Series<'a> {
    rows.iter().copied().filter(|r| r.metric_name == metric && r.class == class).map(|r| (r.bucket, r)).collect()
}

/// Increase of a (summed) cumulative counter in bucket `k`: from the previous bucket's last
/// value when it sums the same series, else within the bucket; `None` after a reset.
fn increase(s: &Series<'_>, k: i64) -> Option<f64> {
    let r = s.get(&k)?;
    let base = match s.range(..k).next_back() {
        Some((_, p)) if p.series == r.series => p.last_value,
        _ => r.first_value,
    };
    let d = r.last_value - base;
    (d >= 0.0).then_some(d)
}

fn share(part: f64, whole: f64) -> Option<f32> {
    (whole > 0.0 && part.is_finite() && whole.is_finite()).then(|| (part / whole).clamp(0.0, 1.0) as f32)
}

fn host_track(id: &str, rows: &[&InfraBucketRow], g: &Geometry) -> HostTrack {
    let busy = series(rows, "system.cpu.time", "busy");
    let idle = series(rows, "system.cpu.time", "idle");
    let util_idle = series(rows, "system.cpu.utilization", "idle");
    let used = series(rows, "system.memory.usage", "used");
    let other = series(rows, "system.memory.usage", "other");
    let limit = series(rows, "system.memory.limit", "");
    let mut cpu = Vec::with_capacity(g.buckets as usize);
    let mut memory = Vec::with_capacity(g.buckets as usize);
    for i in 0..i64::from(g.buckets) {
        let k = i + g.lookback;
        let from_time = match (increase(&busy, k), increase(&idle, k)) {
            (Some(b), Some(d)) => share(b, b + d),
            _ => None,
        };
        let from_gauge = || util_idle.get(&k).and_then(|r| share(r.last_value, r.series as f64)).map(|idle| 1.0 - idle);
        cpu.push(from_time.or_else(from_gauge));
        let used_k = used.get(&k).map(|r| r.last_value);
        let total = limit.get(&k).map(|r| r.last_value).or_else(|| Some(used_k? + other.get(&k)?.last_value));
        memory.push(used_k.zip(total).and_then(|(u, t)| share(u, t)));
    }
    let name = rows.iter().map(|r| r.host_name.as_str()).find(|n| !n.is_empty()).unwrap_or(id).to_owned();
    HostTrack { id: id.to_owned(), name, cpu, memory }
}

fn short(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}

/// One event before redeploys are merged; times in nanoseconds.
struct Found {
    time: i64,
    kind: ContainerEventKind,
    host: String,
    container: String,
    name: String,
    service: Option<String>,
    detail: String,
}

/// Memory share of the limit at its highest in bucket `k` or the one before.
fn memory_peak(rows: &[&InfraBucketRow], k: i64) -> Option<f64> {
    let usage = series(rows, "container.memory.usage.total", "");
    let limit = series(rows, "container.memory.usage.limit", "");
    let peak = usage.range(k - 1..=k).map(|(_, r)| r.max_value).reduce(f64::max)?;
    let limit = limit.range(..=k).next_back().map(|(_, r)| r.last_value)?;
    (limit > 0.0).then(|| peak / limit)
}

fn container_events(
    host_id: &str,
    host_name: &str,
    container_id: &str,
    rows: &[&InfraBucketRow],
    host_span: (i64, i64),
    g: &Geometry,
) -> Vec<Found> {
    let in_range = |t: i64| t >= g.from && t < g.to;
    let name = rows
        .iter()
        .map(|r| r.container_name.trim_start_matches('/'))
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| short(container_id))
        .to_owned();
    let service = rows.iter().map(|r| r.service_name.as_str()).find(|n| !n.is_empty()).map(str::to_owned);
    let event = |time: i64, kind: ContainerEventKind, detail: String| Found {
        time,
        kind,
        host: host_id.to_owned(),
        container: container_id.to_owned(),
        name: name.clone(),
        service: service.clone(),
        detail,
    };
    // A stop or restart with memory at the limit was probably the kernel's OOM killer.
    let maybe_oom = |time: i64, k: i64, kind: ContainerEventKind, verb: &str, plain: String| match memory_peak(rows, k)
    {
        Some(p) if p >= OOM_SHARE => event(
            time,
            ContainerEventKind::OutOfMemory,
            format!("{name} {verb} with memory at {:.0}% of its limit: probably out of memory", p * 100.0),
        ),
        _ => event(time, kind, plain),
    };

    let mut out = Vec::new();
    let first = rows.iter().map(|r| r.first_time).min().unwrap_or(i64::MAX);
    let last = rows.iter().map(|r| r.last_time).max().unwrap_or(i64::MIN);
    if first > host_span.0.saturating_add(STALE_NS) && in_range(first) {
        out.push(event(first, ContainerEventKind::Started, format!("{name} started on {host_name}")));
    }
    if host_span.1 > last.saturating_add(STALE_NS) && in_range(last) {
        let k = rows.iter().map(|r| r.bucket).max().unwrap_or(0);
        out.push(maybe_oom(last, k, ContainerEventKind::Stopped, "stopped", format!("{name} stopped on {host_name}")));
    }
    let cpu = series(rows, "container.cpu.usage.total", "");
    let mut prev: Option<&InfraBucketRow> = None;
    for (&k, &r) in &cpu {
        let reset = match prev {
            Some(p) if r.first_value < p.last_value => Some(r.first_time),
            _ if r.last_value < r.first_value => Some(r.first_time + (r.last_time - r.first_time) / 2),
            _ => None,
        };
        if let Some(t) = reset.filter(|&t| in_range(t)) {
            let plain = format!("{name} restarted on {host_name}");
            out.push(maybe_oom(t, k, ContainerEventKind::Restarted, "restarted", plain));
        }
        prev = Some(r);
    }
    out
}

/// A stop followed by a start of the same name on the same machine is one redeploy.
fn merge_redeploys(found: Vec<Found>, host_names: &HashMap<&str, &str>) -> Vec<Found> {
    let mut stops: Vec<Option<Found>> = Vec::new();
    let mut rest = Vec::new();
    for f in found {
        if f.kind == ContainerEventKind::Stopped {
            stops.push(Some(f));
        } else {
            rest.push(f);
        }
    }
    let mut out = Vec::new();
    for f in rest {
        if f.kind != ContainerEventKind::Started {
            out.push(f);
            continue;
        }
        let old = stops.iter_mut().find(|s| {
            s.as_ref().is_some_and(|s| {
                s.host == f.host
                    && s.name == f.name
                    && s.container != f.container
                    && s.time >= f.time.saturating_sub(REDEPLOY_NS)
                    && s.time <= f.time.saturating_add(STALE_NS)
            })
        });
        match old.and_then(Option::take) {
            Some(old) => {
                let host = host_names.get(f.host.as_str()).copied().unwrap_or(f.host.as_str());
                let detail = format!(
                    "{} redeployed on {host}: container {} replaced {}",
                    f.name,
                    short(&f.container),
                    short(&old.container)
                );
                out.push(Found { kind: ContainerEventKind::Redeployed, detail, ..f });
            }
            None => out.push(f),
        }
    }
    out.extend(stops.into_iter().flatten());
    out
}

pub fn build(g: &Geometry, rows: &[InfraBucketRow], truncated: bool) -> InfraTimeline {
    let mut by_host: BTreeMap<&str, Vec<&InfraBucketRow>> = BTreeMap::new();
    for r in rows.iter().filter(|r| !r.host_id.is_empty()) {
        by_host.entry(r.host_id.as_str()).or_default().push(r);
    }
    let mut hosts = Vec::new();
    let mut found = Vec::new();
    let mut host_names: HashMap<&str, &str> = HashMap::new();
    for (&id, rows) in &by_host {
        let machine: Vec<&InfraBucketRow> = rows.iter().copied().filter(|r| r.container_id.is_empty()).collect();
        let track = host_track(id, if machine.is_empty() { rows } else { &machine }, g);
        let reporting = if machine.is_empty() { rows } else { &machine };
        let span = (
            reporting.iter().map(|r| r.first_time).min().unwrap_or(i64::MAX),
            reporting.iter().map(|r| r.last_time).max().unwrap_or(i64::MIN),
        );
        let mut by_container: BTreeMap<&str, Vec<&InfraBucketRow>> = BTreeMap::new();
        for r in rows.iter().copied().filter(|r| !r.container_id.is_empty()) {
            by_container.entry(r.container_id.as_str()).or_default().push(r);
        }
        for (cid, crows) in &by_container {
            found.extend(container_events(id, &track.name, cid, crows, span, g));
        }
        hosts.push(track);
    }
    for h in &hosts {
        host_names.insert(h.id.as_str(), h.name.as_str());
    }
    let mut found = merge_redeploys(found, &host_names);
    found.sort_by(|a, b| (a.time, &a.host, &a.container).cmp(&(b.time, &b.host, &b.container)));
    let total = found.len();
    let events = found
        .into_iter()
        .take(MAX_EVENTS)
        .map(|f| ContainerEvent {
            time_ns: f.time.to_string(),
            kind: f.kind,
            host_name: host_names.get(f.host.as_str()).map_or_else(|| f.host.clone(), |n| (*n).to_owned()),
            host_id: f.host,
            container_id: f.container,
            container_name: f.name,
            service_name: f.service,
            detail: f.detail,
        })
        .collect();
    hosts.sort_by(|a, b| (a.name.as_str(), a.id.as_str()).cmp(&(b.name.as_str(), b.id.as_str())));
    InfraTimeline {
        from_ns: g.from.to_string(),
        to_ns: g.to.to_string(),
        buckets: g.buckets,
        bucket_ns: g.bucket_ns.to_string(),
        hosts,
        events,
        events_total: u32::try_from(total).unwrap_or(u32::MAX),
        truncated,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const S: i64 = NS_PER_SEC;
    const FROM: i64 = 1_800_000_000 * S;

    /// 10 buckets of 60 s from FROM; 3 look-back buckets.
    fn geometry() -> Geometry {
        Geometry::new(Range { from: FROM, to: FROM + 600 * S }, Some(10))
    }

    struct B {
        host: &'static str,
        container: &'static str,
        name: &'static str,
        metric: &'static str,
        class: &'static str,
        series: u64,
    }

    fn machine(metric: &'static str, class: &'static str) -> B {
        B { host: "vps-app", container: "", name: "", metric, class, series: 1 }
    }

    fn ctr(container: &'static str, name: &'static str, metric: &'static str) -> B {
        B { host: "vps-app", container, name, metric, class: "", series: 1 }
    }

    /// One row in output bucket `i` (negative: look-back) with points 10 s and 50 s into it.
    fn row(b: &B, i: i64, first: f64, last: f64) -> InfraBucketRow {
        let g = geometry();
        let start = g.from + i * g.bucket_ns;
        InfraBucketRow {
            host_id: b.host.into(),
            host_name: b.host.into(),
            container_id: b.container.into(),
            container_name: b.name.into(),
            service_name: String::new(),
            metric_name: b.metric.into(),
            class: b.class.into(),
            bucket: i + g.lookback,
            series: b.series,
            first_value: first,
            last_value: last,
            max_value: first.max(last),
            first_time: start + 10 * S,
            last_time: start + 50 * S,
        }
    }

    /// The machine reports over the whole range and look-back: CPU 25 % busy, memory 512/1024.
    fn host_rows() -> Vec<InfraBucketRow> {
        let busy = B { series: 2, ..machine("system.cpu.time", "busy") };
        let idle = B { series: 2, ..machine("system.cpu.time", "idle") };
        let mut out = Vec::new();
        for i in -3..10 {
            let t = (i + 3) as f64 * 60.0 * 2.0; // two cores: 120 CPU-seconds per bucket
            out.push(row(&busy, i, t * 0.25 - 10.0, t * 0.25));
            out.push(row(&idle, i, t * 0.75 - 30.0, t * 0.75));
            out.push(row(&machine("system.memory.usage", "used"), i, 512.0, 512.0));
            out.push(row(&machine("system.memory.usage", "other"), i, 512.0, 512.0));
        }
        out
    }

    fn kinds(t: &InfraTimeline) -> Vec<(ContainerEventKind, &str)> {
        t.events.iter().map(|e| (e.kind, e.container_name.as_str())).collect()
    }

    #[test]
    fn geometry_clamps_buckets_and_covers_the_look_back() {
        let r = Range { from: FROM, to: FROM + 600 * S };
        assert_eq!(Geometry::new(r, None).buckets, 200);
        assert_eq!(Geometry::new(r, Some(0)).buckets, 1);
        assert_eq!(Geometry::new(r, Some(50_000)).buckets, 1_000);
        let g = geometry();
        assert_eq!((g.bucket_ns, g.lookback), (60 * S, 3));
        assert!(g.from - g.base() >= LOOKBACK_NS);
        // Tiny ranges never divide by zero.
        let tiny = Geometry::new(Range { from: 0, to: 1 }, Some(1000));
        assert!(tiny.bucket_ns >= 1 && tiny.lookback >= 1);
    }

    #[test]
    fn machine_cpu_and_memory_per_bucket() {
        let t = build(&geometry(), &host_rows(), false);
        assert_eq!(t.hosts.len(), 1);
        let h = &t.hosts[0];
        assert_eq!((h.cpu.len(), h.memory.len()), (10, 10));
        for i in 0..10 {
            assert!((h.cpu[i].unwrap() - 0.25).abs() < 1e-4, "bucket {i}: {:?}", h.cpu[i]);
            assert_eq!(h.memory[i], Some(0.5));
        }
        assert!(t.events.is_empty(), "nothing happened to containers: {:?}", t.events);
    }

    #[test]
    fn buckets_without_data_are_null_and_a_counter_reset_gives_no_cpu() {
        let mut rows: Vec<InfraBucketRow> = host_rows().into_iter().filter(|r| r.bucket != 5 + 3).collect();
        // Bucket 7: the busy counter starts over (agent restart).
        for r in rows.iter_mut().filter(|r| r.bucket == 7 + 3 && r.class == "busy") {
            r.first_value = 0.0;
            r.last_value = 1.0;
        }
        let h = &build(&geometry(), &rows, false).hosts[0];
        assert_eq!((h.cpu[5], h.memory[5]), (None, None));
        assert_eq!(h.cpu[7], None);
        assert!(h.cpu[6].is_some(), "a gap before a bucket falls back to its own points");
    }

    #[test]
    fn utilization_gauge_is_the_cpu_fallback_and_memory_limit_wins() {
        let rows = vec![
            row(&B { series: 2, ..machine("system.cpu.utilization", "idle") }, 0, 1.2, 1.2),
            row(&machine("system.memory.usage", "used"), 0, 300.0, 300.0),
            row(&machine("system.memory.limit", ""), 0, 1000.0, 1000.0),
        ];
        let h = &build(&geometry(), &rows, false).hosts[0];
        assert!((h.cpu[0].unwrap() - 0.4).abs() < 1e-6, "idle 60 % on average");
        assert!((h.memory[0].unwrap() - 0.3).abs() < 1e-6);
    }

    #[test]
    fn a_container_that_appears_while_its_machine_reports_has_started() {
        let mut rows = host_rows();
        let cpu = ctr("c1", "/orders", "container.cpu.usage.total");
        rows.extend((-3..10).map(|i| {
            row(&ctr("c0", "inventory", "container.cpu.usage.total"), i, 1.0e9 * (i + 3) as f64, 1.0e9 * (i + 4) as f64)
        }));
        rows.extend((4..10).map(|i| row(&cpu, i, 1.0e9 * i as f64, 1.0e9 * (i + 1) as f64)));
        let t = build(&geometry(), &rows, false);
        assert_eq!(kinds(&t), vec![(ContainerEventKind::Started, "orders")], "inventory ran all along");
        let e = &t.events[0];
        assert_eq!(e.time_ns, (FROM + 4 * 60 * S + 10 * S).to_string());
        assert_eq!((e.host_id.as_str(), e.host_name.as_str(), e.container_id.as_str()), ("vps-app", "vps-app", "c1"));
        assert_eq!(e.detail, "orders started on vps-app");
    }

    #[test]
    fn a_container_that_stops_reporting_while_its_machine_goes_on_has_stopped() {
        let mut rows = host_rows();
        let cpu = ctr("c1", "orders", "container.cpu.usage.total");
        rows.extend((-3..4).map(|i| row(&cpu, i, 1.0e9 * (i + 3) as f64, 1.0e9 * (i + 4) as f64)));
        let t = build(&geometry(), &rows, false);
        assert_eq!(kinds(&t), vec![(ContainerEventKind::Stopped, "orders")]);
        assert_eq!(t.events[0].time_ns, (FROM + 3 * 60 * S + 50 * S).to_string());

        // The whole machine went quiet (agent down): no claim about its containers.
        let quiet: Vec<InfraBucketRow> =
            rows.into_iter().filter(|r| r.bucket < 4 + 3 || !r.container_id.is_empty()).collect();
        assert!(build(&geometry(), &quiet, false).events.is_empty());
    }

    #[test]
    fn a_cpu_counter_that_starts_over_is_a_restart() {
        let mut rows = host_rows();
        let cpu = ctr("c1", "orders", "container.cpu.usage.total");
        for i in -3..10 {
            let (a, b) = match i {
                i if i < 6 => (100.0 + i as f64, 101.0 + i as f64),
                6 => (2.0, 3.0), // reset between buckets 5 and 6
                i => (i as f64, i as f64 + 1.0),
            };
            rows.push(row(&cpu, i, a, b));
        }
        let t = build(&geometry(), &rows, false);
        assert_eq!(kinds(&t), vec![(ContainerEventKind::Restarted, "orders")]);
        assert_eq!(t.events[0].time_ns, (FROM + 6 * 60 * S + 10 * S).to_string());
    }

    #[test]
    fn a_stop_or_restart_at_the_memory_limit_is_probably_out_of_memory() {
        let mut rows = host_rows();
        let cpu = ctr("c1", "orders", "container.cpu.usage.total");
        let mem = ctr("c1", "orders", "container.memory.usage.total");
        let limit = ctr("c1", "orders", "container.memory.usage.limit");
        for i in -3..4 {
            rows.push(row(&cpu, i, (i + 3) as f64, (i + 4) as f64));
            rows.push(row(&mem, i, 100.0 + 100.0 * (i + 3) as f64, 200.0 + 100.0 * (i + 3) as f64));
            rows.push(row(&limit, i, 800.0, 800.0));
        }
        let t = build(&geometry(), &rows, false);
        assert_eq!(kinds(&t), vec![(ContainerEventKind::OutOfMemory, "orders")]);
        assert_eq!(t.events[0].detail, "orders stopped with memory at 100% of its limit: probably out of memory");

        // Same stop with memory well below the limit: a plain stop.
        for r in rows.iter_mut().filter(|r| r.metric_name == "container.memory.usage.limit") {
            r.last_value = 8000.0;
        }
        assert_eq!(kinds(&build(&geometry(), &rows, false)), vec![(ContainerEventKind::Stopped, "orders")]);
    }

    #[test]
    fn a_stop_and_a_start_of_the_same_name_on_the_same_machine_is_one_redeploy() {
        let mut rows = host_rows();
        let old = ctr("aaaaaaaaaaaaaaaa", "orders", "container.cpu.usage.total");
        let new = ctr("bbbbbbbbbbbbbbbb", "orders", "container.cpu.usage.total");
        let other = ctr("cccccccccccccccc", "payments", "container.cpu.usage.total");
        rows.extend((-3..4).map(|i| row(&old, i, (i + 3) as f64, (i + 4) as f64)));
        rows.extend((4..10).map(|i| row(&new, i, i as f64, (i + 1) as f64)));
        rows.extend((5..10).map(|i| row(&other, i, i as f64, (i + 1) as f64)));
        let t = build(&geometry(), &rows, false);
        assert_eq!(
            kinds(&t),
            vec![(ContainerEventKind::Redeployed, "orders"), (ContainerEventKind::Started, "payments")]
        );
        assert_eq!(t.events[0].container_id, "bbbbbbbbbbbbbbbb");
        assert_eq!(t.events[0].detail, "orders redeployed on vps-app: container bbbbbbbbbbbb replaced aaaaaaaaaaaa");
    }

    #[test]
    fn events_are_capped_oldest_first_with_the_total() {
        let mut rows = host_rows();
        // Containers that start in bucket 5 and still run in the last bucket.
        for n in 0..MAX_EVENTS + 20 {
            let id = format!("c{n:04}");
            for (i, a) in [(5, 1.0), (9, 5.0)] {
                let mut r = row(&ctr("", "", "container.cpu.usage.total"), i, a, a + 1.0);
                (r.container_id, r.container_name) = (id.clone(), id.clone());
                r.first_time += n as i64;
                rows.push(r);
            }
        }
        let t = build(&geometry(), &rows, false);
        assert_eq!(t.events.len(), MAX_EVENTS);
        assert_eq!(t.events_total as usize, MAX_EVENTS + 20);
        assert!(
            t.events.windows(2).all(|w| w[0].time_ns.parse::<i64>().unwrap() <= w[1].time_ns.parse::<i64>().unwrap())
        );
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_rows_never_panic_and_shares_stay_in_range(
            raw in proptest::collection::vec((0usize..3, 0usize..7, 0i64..20, -1e12f64..1e12, -1e12f64..1e12, 0u64..4), 0..80),
        ) {
            const METRICS: [(&str, &str); 7] = [
                ("system.cpu.time", "busy"), ("system.cpu.time", "idle"), ("system.cpu.utilization", "idle"),
                ("system.memory.usage", "used"), ("system.memory.limit", ""), ("container.cpu.usage.total", ""),
                ("container.memory.usage.total", ""),
            ];
            let g = geometry();
            let rows: Vec<InfraBucketRow> = raw
                .into_iter()
                .map(|(c, m, k, a, b, series)| {
                    let (metric, class) = METRICS[m];
                    let container = if metric.starts_with("container.") { ["", "c1", "c2"][c] } else { "" };
                    let mut r = row(&B { host: "h", container, name: container, metric, class, series }, k - g.lookback, a, b);
                    r.max_value = a.max(b);
                    r
                })
                .collect();
            let t = build(&g, &rows, false);
            for h in &t.hosts {
                proptest::prop_assert_eq!(h.cpu.len(), 10);
                for v in h.cpu.iter().chain(&h.memory).flatten() {
                    proptest::prop_assert!((0.0..=1.0).contains(v));
                }
            }
            for e in &t.events {
                let time: i64 = e.time_ns.parse().unwrap();
                proptest::prop_assert!(time >= g.from && time < g.to);
            }
            proptest::prop_assert_eq!(t, build(&g, &rows, false));
        }
    }
}
