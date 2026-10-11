use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_cli::discover::node_entry;

fn cli(c: &mut Criterion) {
    let start = "node --import ./instrumentation.mjs --env-file .env.production --conditions=production dist/server.js --port 8080";
    c.bench_function("cli/parse_node_entry", |b| b.iter(|| node_entry(black_box(start))));
}

criterion_group!(benches, cli);
criterion_main!(benches);
