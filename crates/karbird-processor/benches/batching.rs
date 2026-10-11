#![allow(clippy::expect_used)]
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_processor::batching::drain_ready_batch;
use std::collections::VecDeque;

fn processor(c: &mut Criterion) {
    c.bench_function("processor/drain_128_ready_records", |b| {
        b.iter_batched(
            || VecDeque::from((0..127).map(|i| (i, 1_024usize)).collect::<Vec<_>>()),
            |mut queue| {
                let (index, size) = queue.pop_front().expect("fixture has first record");
                drain_ready_batch((index, size), 128, 8 * 1024 * 1024, |item| item.1, || queue.pop_front())
            },
            BatchSize::SmallInput,
        )
    });
    c.bench_function("processor/check_dead_letter_payload_bound", |b| {
        let data = vec![0u8; 256 * 1024];
        b.iter(|| karbird_processor::batching::dead_letter_payload(black_box(&data)))
    });
}

criterion_group!(benches, processor);
criterion_main!(benches);
