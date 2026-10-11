#![allow(clippy::expect_used)]
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_protocol::v1::{CustomEvent, CustomEventBatch, IngestEnvelope, IngestScope, ingest_envelope::Payload};
use prost::Message;

fn envelope() -> IngestEnvelope {
    let events = (0..256)
        .map(|i| CustomEvent {
            event_id: format!("event-{i}"),
            event_type: "deployment.completed".into(),
            source_sequence: Some(i),
            ..Default::default()
        })
        .collect();
    IngestEnvelope {
        schema_version: karbird_protocol::ENVELOPE_SCHEMA_VERSION,
        batch_id: "batch-1".into(),
        scope: Some(IngestScope {
            tenant_id: "tenant-a".into(),
            project_id: "project-a".into(),
            environment_id: "production".into(),
            token_id: "token-a".into(),
        }),
        received_time_unix_nano: 1_790_000_000_000_000_000,
        source_id: "ingest-1".into(),
        payload: Some(Payload::CustomEvents(CustomEventBatch { events })),
    }
}

fn protocol(c: &mut Criterion) {
    let message = envelope();
    let bytes = message.encode_to_vec();
    c.bench_function("protocol/encode_256_events", |b| b.iter(|| black_box(&message).encode_to_vec()));
    c.bench_function("protocol/decode_256_events", |b| {
        b.iter(|| IngestEnvelope::decode(black_box(bytes.as_slice())).expect("fixture decodes"))
    });
}

criterion_group!(benches, protocol);
criterion_main!(benches);
