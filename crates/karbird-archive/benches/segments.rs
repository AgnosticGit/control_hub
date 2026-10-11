#![allow(clippy::expect_used)]
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_archive::{LogRecord, decode, encode};

fn records(count: usize) -> Vec<LogRecord> {
    (0..count)
        .map(|i| LogRecord {
            service_name: "checkout".into(),
            service_instance_id: "checkout-7f9d".into(),
            log_id: format!("log-{i}"),
            time: 1_790_000_000_000_000_000 + i as i64 * 1_000_000,
            observed_time: 1_790_000_000_000_000_000 + i as i64 * 1_000_000,
            severity_number: if i % 97 == 0 { 17 } else { 9 },
            severity_text: "INFO".into(),
            body: format!("order {} accepted by checkout", i % 128),
            trace_id: format!("{:032x}", i / 4),
            span_id: format!("{:016x}", i),
            attributes: vec![("http.route".into(), "/orders/:id".into()), ("status".into(), "accepted".into())],
            resource_attributes: vec![("deployment.environment".into(), "production".into())],
            batch_id: "batch-a".into(),
            ingested_at: 1_790_000_000_100_000_000 + i as i64 * 1_000_000,
        })
        .collect()
}

fn archive(c: &mut Criterion) {
    let input = records(512);
    let encoded = encode(&input, None, 3).expect("fixture encodes");
    c.bench_function("archive/encode_512_logs", |b| {
        b.iter_batched(
            || input.clone(),
            |rows| encode(black_box(&rows), None, 3).expect("encode"),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("archive/decode_512_logs", |b| {
        b.iter(|| decode(black_box(&encoded.bytes), None).expect("decode"))
    });
}

criterion_group!(benches, archive);
criterion_main!(benches);
