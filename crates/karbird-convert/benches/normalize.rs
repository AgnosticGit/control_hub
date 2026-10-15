#![allow(clippy::expect_used)]
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_convert::convert;
use karbird_protocol::otlp::collector::logs::v1::ExportLogsServiceRequest;
use karbird_protocol::otlp::collector::metrics::v1::ExportMetricsServiceRequest;
use karbird_protocol::otlp::collector::trace::v1::ExportTraceServiceRequest;
use karbird_protocol::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
use karbird_protocol::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use karbird_protocol::otlp::metrics::v1::{
    Gauge, Histogram, HistogramDataPoint, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, metric::Data,
    number_data_point,
};
use karbird_protocol::otlp::resource::v1::Resource;
use karbird_protocol::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};
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

fn histogram_conversion_fixture() -> IngestEnvelope {
    let request = ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(Resource { attributes: vec![kv("service.name", "orders")], ..Default::default() }),
            scope_metrics: vec![ScopeMetrics {
                metrics: vec![Metric {
                    name: "http.server.request.duration".into(),
                    unit: "s".into(),
                    data: Some(Data::Histogram(Histogram {
                        aggregation_temporality: 2,
                        data_points: (0..256)
                            .map(|i| HistogramDataPoint {
                                time_unix_nano: 1_790_000_000_000_000_000 + i,
                                count: 100,
                                sum: Some(2.5),
                                explicit_bounds: vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5],
                                bucket_counts: vec![5, 20, 25, 20, 15, 10, 3, 2, 0],
                                attributes: vec![kv("http.route", "/orders/{id}")],
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
    let mut env = metric_conversion_fixture();
    env.payload = Some(Payload::OtlpMetrics(request.encode_to_vec()));
    env
}

fn trace_conversion_fixture() -> IngestEnvelope {
    let request = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![kv("service.name", "orders"), kv("deployment.environment", "production")],
                ..Default::default()
            }),
            scope_spans: vec![ScopeSpans {
                spans: (0u64..1_000)
                    .map(|i| Span {
                        trace_id: vec![1; 16],
                        span_id: (i + 1).to_be_bytes().to_vec(),
                        name: format!("POST /orders/{i}"),
                        kind: 2,
                        start_time_unix_nano: 1_790_000_000_000_000_000 + i,
                        end_time_unix_nano: 1_790_000_000_000_001_000 + i,
                        attributes: vec![kv("http.request.method", "POST"), kv("http.route", "/orders/:id")],
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
        batch_id: "trace-batch".into(),
        scope: Some(IngestScope {
            tenant_id: "tenant-a".into(),
            project_id: "project-a".into(),
            environment_id: "production".into(),
            token_id: "token-a".into(),
        }),
        received_time_unix_nano: 1_790_000_000_000_002_000,
        source_id: "ingest-1".into(),
        payload: Some(Payload::OtlpTraces(request.encode_to_vec())),
    }
}

fn log_conversion_fixture() -> IngestEnvelope {
    let request = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(Resource {
                attributes: vec![kv("service.name", "orders"), kv("deployment.environment", "production")],
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
                        attributes: vec![kv("http.route", "/orders/:id"), kv("order.status", "accepted")],
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

fn normalization(c: &mut Criterion) {
    let input = convert_fixture();
    c.bench_function("convert/normalize_256_custom_events", |b| {
        b.iter(|| convert(black_box(&input), black_box(1_790_000_000_000_000_001)).expect("fixture converts"))
    });

    let metrics = metric_conversion_fixture();
    c.bench_function("convert/normalize_256_metric_points", |b| {
        b.iter(|| convert(black_box(&metrics), black_box(1_790_000_000_000_000_001)).expect("fixture converts"))
    });

    let histograms = histogram_conversion_fixture();
    c.bench_function("convert/normalize_256_histograms_9_buckets", |b| {
        b.iter(|| convert(black_box(&histograms), black_box(1_790_000_000_000_000_001)).expect("fixture converts"))
    });

    let traces = trace_conversion_fixture();
    c.bench_function("convert/normalize_1000_spans", |b| {
        b.iter(|| convert(black_box(&traces), black_box(1_790_000_000_000_002_001)).expect("fixture converts"))
    });

    let logs = log_conversion_fixture();
    c.bench_function("convert/normalize_1000_logs", |b| {
        b.iter(|| convert(black_box(&logs), black_box(1_790_000_000_000_002_001)).expect("fixture converts"))
    });
}

criterion_group!(benches, normalization);
criterion_main!(benches);
