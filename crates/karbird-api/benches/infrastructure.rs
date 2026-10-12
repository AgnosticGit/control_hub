use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_api::routes::infrastructure::{hosts_from_series, place_resources};
use karbird_engine_core::InfrastructureSnapshot;
use karbird_storage::ch::{InfraSeriesRow, ResourceAddressRow};

fn series(hosts: usize) -> Vec<InfraSeriesRow> {
    (0..hosts)
        .flat_map(|i| {
            let host_id = format!("host-{i:04}");
            [
                InfraSeriesRow {
                    host_id: host_id.clone(),
                    host_name: format!("node-{i:04}"),
                    container_id: String::new(),
                    container_name: String::new(),
                    container_hostname: String::new(),
                    container_image: String::new(),
                    container_service: String::new(),
                    service_name: String::new(),
                    metric_name: "system.cpu.utilization".into(),
                    series_id: i as u64,
                    attributes: vec![],
                    first_value: 0.25,
                    first_time: 1_790_000_000,
                    last_value: 0.7,
                    last_time: 1_790_000_060,
                },
                InfraSeriesRow {
                    host_id: host_id.clone(),
                    host_name: format!("node-{i:04}"),
                    container_id: format!("container-{i:04}"),
                    container_name: format!("orders-{i:04}"),
                    container_hostname: format!("orders-{i:04}"),
                    container_image: "karbird/orders:v1".into(),
                    container_service: "orders".into(),
                    service_name: "orders".into(),
                    metric_name: "container.cpu.usage.total".into(),
                    series_id: i as u64 + hosts as u64,
                    attributes: vec![],
                    first_value: 10.0,
                    first_time: 1_790_000_000,
                    last_value: 20.0,
                    last_time: 1_790_000_060,
                },
            ]
        })
        .collect()
}

fn metric_series(
    host_id: &str,
    container_id: &str,
    metric_name: &str,
    attributes: &[(&str, &str)],
    first_value: f64,
    last_value: f64,
) -> InfraSeriesRow {
    let is_container = !container_id.is_empty();
    InfraSeriesRow {
        host_id: host_id.into(),
        host_name: format!("node-{host_id}"),
        container_id: container_id.into(),
        container_name: if is_container { format!("orders-{host_id}") } else { String::new() },
        container_hostname: String::new(),
        container_image: if is_container { "karbird/orders:v1".into() } else { String::new() },
        container_service: if is_container { "orders".into() } else { String::new() },
        service_name: if is_container { "orders".into() } else { String::new() },
        metric_name: metric_name.into(),
        series_id: 1,
        attributes: attributes.iter().map(|(key, value)| ((*key).into(), (*value).into())).collect(),
        first_value,
        first_time: 1_790_000_000,
        last_value,
        last_time: 1_790_000_060,
    }
}

fn realistic_series(hosts: usize) -> Vec<InfraSeriesRow> {
    let mut rows = Vec::with_capacity(hosts * 31);
    for i in 0..hosts {
        let host_id = format!("host-{i:04}");
        for cpu in 0..4 {
            let cpu_id = cpu.to_string();
            for (state, seconds) in [("user", 14.0), ("system", 1.0), ("idle", 44.0), ("wait", 1.0)] {
                rows.push(metric_series(
                    &host_id,
                    "",
                    "system.cpu.time",
                    &[("cpu", cpu_id.as_str()), ("state", state)],
                    0.0,
                    seconds,
                ));
            }
        }
        rows.push(metric_series(&host_id, "", "system.cpu.logical.count", &[], 0.0, 4.0));
        rows.push(metric_series(&host_id, "", "system.memory.usage", &[("state", "used")], 0.0, 870.0 * 1_048_576.0));
        rows.push(metric_series(&host_id, "", "system.memory.usage", &[("state", "free")], 0.0, 154.0 * 1_048_576.0));
        rows.push(metric_series(&host_id, "", "system.memory.limit", &[], 0.0, 1024.0 * 1_048_576.0));
        for (mountpoint, state, value) in
            [("/", "used", 20e9), ("/", "free", 30e9), ("/data", "used", 80e9), ("/data", "free", 20e9)]
        {
            rows.push(metric_series(
                &host_id,
                "",
                "system.filesystem.usage",
                &[("mountpoint", mountpoint), ("state", state)],
                0.0,
                value,
            ));
        }
        for (direction, value) in [("receive", 6_000_000.0), ("transmit", 3_000_000.0)] {
            rows.push(metric_series(
                &host_id,
                "",
                "system.network.io",
                &[("device", "eth0"), ("direction", direction)],
                0.0,
                value,
            ));
        }
        let container_id = format!("container-{i:04}");
        rows.push(metric_series(&host_id, &container_id, "container.cpu.usage.total", &[], 0.0, 57e9));
        rows.push(metric_series(
            &host_id,
            &container_id,
            "container.memory.usage.total",
            &[],
            0.0,
            300.0 * 1_048_576.0,
        ));
        rows.push(metric_series(
            &host_id,
            &container_id,
            "container.memory.usage.limit",
            &[],
            0.0,
            1024.0 * 1_048_576.0,
        ));
        rows.push(metric_series(&host_id, &container_id, "container.network.io.usage.rx_bytes", &[], 0.0, 600.0));
        rows.push(metric_series(&host_id, &container_id, "container.network.io.usage.tx_bytes", &[], 0.0, 300.0));
    }
    rows
}

fn utilization_fallback_series(hosts: usize) -> Vec<InfraSeriesRow> {
    let mut rows = Vec::with_capacity(hosts * 4);
    for i in 0..hosts {
        let host_id = format!("host-{i:04}");
        for cpu in 0..4 {
            let cpu_id = cpu.to_string();
            rows.push(metric_series(
                &host_id,
                "",
                "system.cpu.utilization",
                &[("cpu", cpu_id.as_str()), ("state", "idle")],
                0.0,
                0.1,
            ));
        }
    }
    rows
}

fn resource_placement_fixture(hosts: usize) -> (InfrastructureSnapshot, Vec<InfraSeriesRow>, Vec<ResourceAddressRow>) {
    let mut rows = realistic_series(hosts);
    let mut addresses = Vec::with_capacity(hosts * 3);
    for i in 0..hosts {
        let host_id = format!("host-{i:04}");
        let container_id = format!("postgres-{i:04}");
        let mut db = metric_series(&host_id, &container_id, "container.cpu.usage.total", &[], 0.0, 1.0e9);
        db.container_name = format!("postgres-{i:04}");
        db.container_hostname = format!("postgres-{i:04}");
        db.container_image = "postgres:16-alpine".into();
        db.container_service = "db".into();
        rows.push(db);

        for _ in 0..3 {
            addresses.push(ResourceAddressRow {
                target_entity_id: format!("resource-{i:04}"),
                target_name: "postgresql: primary".into(),
                target_kind: "database".into(),
                address: "db".into(),
                port: "5432".into(),
                caller_service_id: format!("orders-{i:04}"),
                caller_host_name: format!("node-{host_id}"),
                caller_host_id: host_id.clone(),
                caller_container_id: format!("container-{i:04}"),
                calls: 10,
            });
        }
    }
    let snapshot = InfrastructureSnapshot { hosts: hosts_from_series(&rows), ..InfrastructureSnapshot::default() };
    (snapshot, rows, addresses)
}

fn api(c: &mut Criterion) {
    let rows = series(500);
    c.bench_function("api/group_500_hosts_and_containers", |b| b.iter(|| hosts_from_series(black_box(&rows))));
    let realistic = realistic_series(500);
    c.bench_function("api/group_500_hosts_realistic_metrics", |b| b.iter(|| hosts_from_series(black_box(&realistic))));
    let utilization_fallback = utilization_fallback_series(500);
    c.bench_function("api/group_500_hosts_cpu_utilization_fallback", |b| {
        b.iter(|| hosts_from_series(black_box(&utilization_fallback)))
    });

    let (snapshot, placement_series, addresses) = resource_placement_fixture(200);
    c.bench_function("api/place_200_resources_600_addresses", |b| {
        b.iter_batched(
            || snapshot.clone(),
            |mut snapshot| place_resources(&mut snapshot, black_box(&placement_series), black_box(&addresses)),
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, api);
criterion_main!(benches);
