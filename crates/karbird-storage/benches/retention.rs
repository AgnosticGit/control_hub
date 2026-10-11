use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_storage::retention::parse_ttl_days;

fn storage(c: &mut Criterion) {
    let engine = "ReplacingMergeTree(ingested_at) PARTITION BY toYYYYMMDD(time) ORDER BY (tenant_id, project_id, time) TTL toDateTime(time) + toIntervalDay(365) SETTINGS index_granularity = 8192";
    c.bench_function("storage/parse_clickhouse_ttl", |b| {
        b.iter(|| parse_ttl_days(black_box(engine), black_box("time")))
    });
}

criterion_group!(benches, storage);
criterion_main!(benches);
