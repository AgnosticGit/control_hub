#![allow(clippy::expect_used)]

use divan::{AllocProfiler, black_box};
use karbird_protocol::v1::{CustomEvent, CustomEventBatch, IngestEnvelope, IngestScope, ingest_envelope::Payload};
use prost::Message;

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
}

fn fixture() -> IngestEnvelope {
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

#[divan::bench]
fn encode_256_events() -> Vec<u8> {
    black_box(&fixture()).encode_to_vec()
}
