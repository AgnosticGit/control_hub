use divan::{AllocProfiler, black_box};
use karbird_api::routes::infrastructure::{hosts_from_series, place_resources};
use karbird_engine_core::InfrastructureSnapshot;
use karbird_storage::ch::{InfraSeriesRow, ResourceAddressRow};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
}

fn fixture(host_count: usize) -> Vec<InfraSeriesRow> {
    (0..host_count)
        .map(|i| InfraSeriesRow {
            host_id: format!("host-{i:04}"),
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
        })
        .collect()
}

fn placement_fixture() -> (InfrastructureSnapshot, Vec<InfraSeriesRow>, Vec<ResourceAddressRow>) {
    let mut series = Vec::with_capacity(600);
    let mut addresses = Vec::with_capacity(600);
    for index in 0..200 {
        let host_id = format!("host-{index:04}");
        let caller_id = format!("orders-{index:04}");
        series.push(InfraSeriesRow {
            host_id: host_id.clone(),
            host_name: format!("node-{index:04}"),
            container_id: String::new(),
            container_name: String::new(),
            container_hostname: String::new(),
            container_image: String::new(),
            container_service: String::new(),
            service_name: String::new(),
            metric_name: "system.cpu.utilization".into(),
            series_id: index as u64,
            attributes: Vec::new(),
            first_value: 0.2,
            first_time: 1_790_000_000,
            last_value: 0.7,
            last_time: 1_790_000_060,
        });
        for (offset, (container_id, service, image)) in
            [(caller_id.as_str(), caller_id.as_str(), "karbird/orders:v1"), ("postgres", "db", "postgres:16-alpine")]
                .into_iter()
                .enumerate()
        {
            series.push(InfraSeriesRow {
                host_id: host_id.clone(),
                host_name: format!("node-{index:04}"),
                container_id: format!("{container_id}-{index:04}"),
                container_name: format!("{container_id}-{index:04}"),
                container_hostname: format!("{container_id}-{index:04}"),
                container_image: image.into(),
                container_service: service.into(),
                service_name: service.into(),
                metric_name: "container.cpu.usage.total".into(),
                series_id: index as u64 + 200 + offset as u64 * 200,
                attributes: Vec::new(),
                first_value: 1.0,
                first_time: 1_790_000_000,
                last_value: 2.0,
                last_time: 1_790_000_060,
            });
        }
        for _ in 0..3 {
            addresses.push(ResourceAddressRow {
                target_entity_id: format!("resource-{index:04}"),
                target_name: "postgresql: primary".into(),
                target_kind: "database".into(),
                address: "db".into(),
                port: "5432".into(),
                caller_service_id: "orders".into(),
                caller_host_name: format!("node-{index:04}"),
                caller_host_id: host_id.clone(),
                caller_container_id: format!("orders-{index:04}"),
                calls: 10,
            });
        }
    }
    let snapshot = InfrastructureSnapshot { hosts: hosts_from_series(&series), ..InfrastructureSnapshot::default() };
    (snapshot, series, addresses)
}

#[divan::bench]
fn group_500_hosts() -> Vec<karbird_engine_core::HostDto> {
    let rows = fixture(500);
    hosts_from_series(black_box(&rows))
}

#[divan::bench]
fn place_200_resources_600_addresses(bencher: divan::Bencher) {
    bencher.with_inputs(placement_fixture).bench_values(|(mut snapshot, series, addresses)| {
        place_resources(&mut snapshot, black_box(&series), black_box(&addresses));
        black_box(snapshot.resources.len())
    });
}
