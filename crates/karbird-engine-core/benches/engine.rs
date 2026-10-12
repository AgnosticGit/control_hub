#![allow(clippy::expect_used)]
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_engine_core::{
    Batch, Engine, EngineConfig, EntityKindDto, Filters, ProvenanceDto, RelationKindDto, SpanItem, TimelineItem,
    TopologyEdge, TopologyNode, TopologySnapshot, build_server_geometry,
};

fn fixture(count: usize) -> Vec<TimelineItem> {
    (0..count)
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
        .collect()
}

fn search_fixture(count: usize) -> Engine {
    let items = (0..count)
        .map(|i| {
            let service = format!("svc-{i:04}");
            TimelineItem::Span(SpanItem {
                trace_id: format!("{i:032x}"),
                span_id: format!("{i:016x}"),
                parent_span_id: None,
                service_id: service,
                service_name: format!("Service {i:04}"),
                target_entity_id: None,
                source_entity_id: None,
                name: "GET /health".into(),
                kind: 2,
                start_ns: 1_790_000_000_000_000_000 + i as i64,
                end_ns: 1_790_000_000_000_001_000 + i as i64,
                is_error: false,
            })
        })
        .collect();
    let mut engine = Engine::new(EngineConfig::default());
    engine.apply_batch(Batch { items }).expect("search fixture applies");
    engine.set_filters(Filters { search: "service 0099".into(), ..Filters::default() });
    engine.seek(1_790_000_000_000_000_500, 1);
    black_box(engine.visible_state());
    engine
}

fn topology_fixture(count: usize) -> TopologySnapshot {
    let nodes: Vec<_> = (0..count)
        .map(|i| TopologyNode {
            id: format!("svc-{i:04}"),
            name: format!("Service {i:04}"),
            kind: EntityKindDto::Service,
            provenance: ProvenanceDto::Observed,
        })
        .collect();
    let mut edges = Vec::with_capacity(count.saturating_sub(1) * 2);
    for i in 0..count.saturating_sub(1) {
        for step in [1, 3] {
            if i + step < count {
                edges.push(TopologyEdge {
                    source: nodes[i].id.clone(),
                    target: nodes[i + step].id.clone(),
                    relation: RelationKindDto::Calls,
                    provenance: ProvenanceDto::Observed,
                    reasons: Vec::new(),
                });
            }
        }
    }
    TopologySnapshot { nodes, edges }
}

fn engine(c: &mut Criterion) {
    let items = fixture(2_000);
    c.bench_function("engine/apply_batch_2000_spans", |b| {
        b.iter_batched(
            || (Engine::new(EngineConfig::default()), items.clone()),
            |(mut engine, batch)| engine.apply_batch(black_box(Batch { items: batch })).expect("fixture applies"),
            BatchSize::LargeInput,
        )
    });
    let mut loaded = Engine::new(EngineConfig::default());
    loaded.apply_batch(Batch { items }).expect("fixture applies");
    c.bench_function("engine/seek_and_build_visible_state", |b| {
        let mut generation = 1u64;
        b.iter(|| {
            generation += 1;
            loaded.seek(black_box(1_790_000_001_000_000_000), generation);
            black_box(loaded.visible_state())
        })
    });
    let batch_json = serde_json::to_string(&Batch { items: fixture(2_000) }).expect("fixture serializes");
    c.bench_function("engine/apply_batch_json_2000_spans", |b| {
        b.iter_batched(
            || Engine::new(EngineConfig::default()),
            |mut engine| engine.apply_batch_json(black_box(&batch_json)).expect("fixture applies"),
            BatchSize::SmallInput,
        )
    });
    let mut searched = search_fixture(1_000);
    c.bench_function("engine/search_visible_state_1000_nodes", |b| {
        let mut generation = 1u64;
        b.iter(|| {
            generation += 1;
            searched.seek(black_box(1_790_000_000_000_000_500), generation);
            black_box(searched.visible_state())
        })
    });
    let topology = topology_fixture(100);
    c.bench_function("engine/build_server_geometry_100_nodes", |b| {
        b.iter(|| build_server_geometry(black_box(&topology), None, None).expect("fixture lays out"))
    });
}

criterion_group!(benches, engine);
criterion_main!(benches);
