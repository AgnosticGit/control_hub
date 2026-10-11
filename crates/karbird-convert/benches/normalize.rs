#![allow(clippy::expect_used)]
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_convert::convert;
use karbird_protocol::otlp::collector::metrics::v1::ExportMetricsServiceRequest;
use karbird_protocol::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
use karbird_protocol::otlp::metrics::v1::{
    Gauge, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, metric::Data, number_data_point,
};
use karbird_protocol::otlp::resource::v1::Resource;
use karbird_protocol::v1::{CustomEvent, CustomEventBatch, IngestEnvelope, IngestScope, ingest_envelope::Payload};
use prost::Message;

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue { value: Some(Value::StringValue(value.into())) }),
        ..Default::default()
    }
}

fn convert_fixture() -> IngestEnvelope {
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

fn metric_conversion_fixture() -> IngestEnvelope {
    let request = ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(Resource {
                attributes: vec![
                    kv("service.name", "orders"),
                    kv("host.name", "vps-app"),
                    kv("deployment.environment", "production"),
                    kv("cloud.region", "eu-frankfurt-1"),
                    kv("container.id", "a1b2c3d4e5f6"),
                    kv("container.name", "orders-1"),
                    kv("container.image.name", "karbird/orders:v1"),
                    kv("docker.compose.service", "orders"),
                ],
                ..Default::default()
            }),
            scope_metrics: vec![ScopeMetrics {
                metrics: vec![Metric {
                    name: "system.cpu.utilization".into(),
                    unit: "%".into(),
                    data: Some(Data::Gauge(Gauge {
                        data_points: (0..256)
                            .map(|i| NumberDataPoint {
                                time_unix_nano: 1_790_000_000_000_000_000 + i,
                                value: Some(number_data_point::Value::AsDouble((i % 100) as f64 / 100.0)),
                                attributes: vec![
                                    kv("cpu.mode", "user"),
                                    kv("cpu.logical_number", &(i % 8).to_string()),
                                    kv("state", "active"),
                                    kv("unit", "percent"),
                                ],
                                ..Default::default()
                            })
                            .collect(),
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
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
        payload: Some(Payload::OtlpMetrics(request.encode_to_vec())),
    }
}

fn normalization(c: &mut Criterion) {
    let input = convert_fixture();
    c.bench_function("convert/normalize_256_custom_events", |b| {
        b.iter(|| convert(black_box(&input), black_box(1_790_000_000_000_000_001)).expect("fixture converts"))
    });

    let metrics = metric_conversion_fixture();
    c.bench_function("convert/normalize_256_metric_points", |b| {
        b.iter(|| convert(black_box(&metrics), black_box(1_790_000_000_000_000_001)).expect("fixture converts"))
    });
}

criterion_group!(benches, normalization);
criterion_main!(benches);
