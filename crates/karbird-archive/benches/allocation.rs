#![allow(clippy::expect_used)]

use divan::{AllocProfiler, black_box};
use karbird_archive::{LogRecord, encode};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
}

fn records(count: usize) -> Vec<LogRecord> {
    (0..count)
        .map(|i| LogRecord {
            service_name: "checkout".into(),
            service_instance_id: "checkout-7f9d".into(),
            log_id: format!("log-{i}"),
            time: 1_790_000_000_000_000_000 + i as i64 * 1_000_000,
            observed_time: 1_790_000_000_000_000_000 + i as i64 * 1_000_000,
            severity_number: 9,
            severity_text: "INFO".into(),
            body: format!("order {} accepted", i % 128),
            trace_id: format!("{:032x}", i / 4),
            span_id: format!("{:016x}", i),
            attributes: vec![("http.route".into(), "/orders/:id".into())],
            resource_attributes: vec![("deployment.environment".into(), "production".into())],
            batch_id: "batch-a".into(),
            ingested_at: 1_790_000_000_100_000_000 + i as i64 * 1_000_000,
        })
        .collect()
}

#[divan::bench]
fn encode_512_logs() -> Vec<u8> {
    encode(black_box(&records(512)), None, 3).expect("benchmark fixture encodes").bytes
}
