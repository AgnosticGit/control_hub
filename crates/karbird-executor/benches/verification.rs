use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_actions::{Observed, Verification};
use karbird_executor::verification::verification_record;

fn executor(c: &mut Criterion) {
    let expected = Verification {
        service: "orders",
        instance: None,
        observe_secs: 30,
        min_requests: 100,
        max_error_ratio: Some(0.01),
        expect_instances: Some(3),
        expect_version: None,
    };
    let observed = Observed { requests: 2_000, errors: 3, instances: 3, other_version_requests: 0 };
    c.bench_function("executor/evaluate_and_record_verification", |b| {
        b.iter(|| verification_record(black_box(&expected), black_box(&observed), 1_790_000_000_000, 1_790_000_030_000))
    });
}

criterion_group!(benches, executor);
criterion_main!(benches);
