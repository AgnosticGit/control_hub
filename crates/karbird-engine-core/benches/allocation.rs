#![allow(clippy::expect_used)]

use divan::{AllocProfiler, black_box};
use karbird_engine_core::{Batch, Engine, EngineConfig, SpanItem, TimelineItem};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
}

fn fixture(count: usize) -> Batch {
    Batch {
        items: (0..count)
            .map(|i| {
                let service = format!("svc-{}", i % 24);
                TimelineItem::Span(SpanItem {
                    trace_id: format!("{:032x}", i / 2),
                    span_id: format!("{:016x}", i),
                    parent_span_id: (i % 2 == 1).then(|| format!("{:016x}", i - 1)),
                    service_id: service.clone(),
                    service_name: service,
                    target_entity_id: None,
                    source_entity_id: None,
                    name: "GET /orders/:id".into(),
                    kind: 2,
                    start_ns: 1_790_000_000_000_000_000 + i as i64 * 1_000_000,
                    end_ns: 1_790_000_000_010_000_000 + i as i64 * 1_000_000,
                    is_error: i % 41 == 0,
                })
            })
            .collect(),
    }
}

#[divan::bench]
fn apply_batch_2000_spans() -> usize {
    let mut engine = Engine::new(EngineConfig::default());
    engine.apply_batch(black_box(fixture(2_000))).expect("benchmark fixture applies");
    engine.visible_state().node_data.len()
}
