#![allow(clippy::expect_used)]

use divan::{AllocProfiler, black_box};
use karbird_convert::convert;
use karbird_protocol::otlp::collector::logs::v1::ExportLogsServiceRequest;
use karbird_protocol::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
use karbird_protocol::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use karbird_protocol::otlp::resource::v1::Resource;
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

fn logs_fixture() -> IngestEnvelope {
    let request = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(Resource {
                attributes: vec![KeyValue {
                    key: "service.name".into(),
                    value: Some(AnyValue { value: Some(Value::StringValue("orders".into())) }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            scope_logs: vec![ScopeLogs {
                log_records: (0..1_000)
                    .map(|i| LogRecord {
                        time_unix_nano: 1_790_000_000_000_000_000 + i,
                        observed_time_unix_nano: 1_790_000_000_000_001_000 + i,
                        severity_number: 9,
                        severity_text: "INFO".into(),
                        body: Some(AnyValue { value: Some(Value::StringValue(format!("order {i} accepted"))) }),
                        attributes: vec![
                            KeyValue {
                                key: "http.route".into(),
                                value: Some(AnyValue { value: Some(Value::StringValue("/orders/:id".into())) }),
                                ..Default::default()
                            },
                            KeyValue {
                                key: "order.status".into(),
                                value: Some(AnyValue { value: Some(Value::StringValue("accepted".into())) }),
                                ..Default::default()
                            },
                        ],
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    IngestEnvelope {
        schema_version: karbird_protocol::ENVELOPE_SCHEMA_VERSION,
        batch_id: "log-batch".into(),
        scope: Some(IngestScope {
            tenant_id: "tenant-a".into(),
            project_id: "project-a".into(),
            environment_id: "production".into(),
            token_id: "token-a".into(),
        }),
        received_time_unix_nano: 1_790_000_000_000_002_000,
        source_id: "ingest-1".into(),
        payload: Some(Payload::OtlpLogs(request.encode_to_vec())),
    }
}

#[divan::bench]
fn convert_256_custom_events() -> karbird_convert::Converted {
    convert(black_box(&fixture()), 1_790_000_000_100_000_000).expect("benchmark fixture converts")
}

#[divan::bench]
fn convert_1000_logs(bencher: divan::Bencher) {
    bencher.with_inputs(logs_fixture).bench_values(|input| {
        let result = convert(black_box(&input), 1_790_000_000_100_000_000).expect("log fixture converts");
        black_box(result.logs.len())
    });
}
