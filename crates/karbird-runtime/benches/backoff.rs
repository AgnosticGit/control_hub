use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_runtime::backoff;
use std::time::Duration;

fn runtime(c: &mut Criterion) {
    c.bench_function("runtime/backoff_schedule", |b| {
        b.iter(|| backoff(black_box(7), Duration::from_millis(200), Duration::from_secs(10)))
    });
}

criterion_group!(benches, runtime);
criterion_main!(benches);
