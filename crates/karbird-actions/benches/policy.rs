#![allow(clippy::expect_used)]
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_actions::{PolicyInput, evaluate, validate};
use serde_json::json;

fn actions(c: &mut Criterion) {
    let params = json!({"replicas": 3});
    let action = validate("demo.replicas.scale", "catalog", &params).expect("benchmark fixture is valid");
    let input = PolicyInput { environment_is_demo: true, open_plans_for_target: 1, approved_by_proposer: false };
    c.bench_function("actions/validate_scale", |b| {
        b.iter(|| validate(black_box("demo.replicas.scale"), black_box("catalog"), black_box(&params)))
    });
    c.bench_function("actions/evaluate_policy", |b| b.iter(|| evaluate(black_box(&action), black_box(&input), false)));
}

criterion_group!(benches, actions);
criterion_main!(benches);
