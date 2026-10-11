#![allow(clippy::expect_used)]
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_web_engine::WebEngine;

fn web_engine(c: &mut Criterion) {
    let config = r#"{"max_items":200000,"rolling_window_ns":"3600000000000","hop_before_ns":"300000000000","hop_after_ns":"300000000000"}"#;
    c.bench_function("web_engine/construct_from_config_json", |b| {
        b.iter(|| WebEngine::new(black_box(config)).expect("fixture config is valid"))
    });
}

criterion_group!(benches, web_engine);
criterion_main!(benches);
