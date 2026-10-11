use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_archiver::plan::{PlanInput, plan};

fn archiver(c: &mut Criterion) {
    let input = PlanInput {
        now: 1_790_000_000_000_000_000,
        window: 3_600_000_000_000,
        min_age: 3_600_000_000_000,
        hot_retention: 14 * 24 * 3_600_000_000_000,
        safety: 3_600_000_000_000,
        earliest_data: Some(1_789_000_000_000_000_000),
        last_done_end: Some(1_789_500_000_000_000_000),
        max_windows: 500,
    };
    c.bench_function("archiver/plan_closed_windows", |b| b.iter(|| plan(black_box(&input))));
}

criterion_group!(benches, archiver);
criterion_main!(benches);
