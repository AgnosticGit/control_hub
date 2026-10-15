use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use karbird_ingest::sanitize::{sanitize_logs, sanitize_metrics, sanitize_traces};
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value};
use opentelemetry_proto::tonic::logs::v1::{LogRecord as OtlpLogRecord, ResourceLogs, ScopeLogs};
use opentelemetry_proto::tonic::metrics::v1::{
    Gauge, Histogram, HistogramDataPoint, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, metric::Data,
    number_data_point,
};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue { value: Some(Value::StringValue(value.into())) }),
        ..Default::default()
    }
}

fn traces() -> ExportTraceServiceRequest {
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans: (0..1_000)
                    .map(|i| Span {
                        trace_id: vec![((i % 255) + 1) as u8; 16],
                        span_id: vec![((i % 255) + 1) as u8; 8],
                        name: format!("GET /orders/{i}"),
                        start_time_unix_nano: 1_790_000_000_000_000_000,
                        end_time_unix_nano: 1_790_000_001_000_000_000,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn traces_with_attributes() -> ExportTraceServiceRequest {
    let mut req = traces();
    for span in &mut req.resource_spans[0].scope_spans[0].spans {
        span.attributes = vec![
            KeyValue {
                key: "http.request.method".into(),
                value: Some(AnyValue { value: Some(Value::StringValue("GET".into())) }),
                ..Default::default()
            },
            KeyValue {
                key: "http.route".into(),
                value: Some(AnyValue { value: Some(Value::StringValue("/orders/:id".into())) }),
                ..Default::default()
            },
            KeyValue {
                key: "http.request.header.authorization".into(),
                value: Some(AnyValue { value: Some(Value::StringValue("Bearer abc.def-123".into())) }),
                ..Default::default()
            },
            KeyValue {
                key: "url.full".into(),
                value: Some(AnyValue {
                    value: Some(Value::StringValue("https://api.example/orders/123?token=secret".into())),
                }),
                ..Default::default()
            },
        ];
    }
    req
}

fn metrics_with_attributes() -> ExportMetricsServiceRequest {
    ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(Resource {
                attributes: vec![
                    kv("service.name", "orders"),
                    kv("host.name", "vps-app"),
                    kv("deployment.environment", "production"),
                    kv("cloud.region", "eu-frankfurt-1"),
                    kv("cloud.provider", "oracle"),
                    kv("container.name", "orders-1"),
                    kv("process.runtime.name", "rust"),
                    kv("telemetry.sdk.language", "rust"),
                ],
                ..Default::default()
            }),
            scope_metrics: vec![ScopeMetrics {
                metrics: vec![Metric {
                    name: "system.cpu.utilization".into(),
                    unit: "%".into(),
                    data: Some(Data::Gauge(Gauge {
                        data_points: (0..1_000)
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
    }
}

fn histograms_with_attributes() -> ExportMetricsServiceRequest {
    ExportMetricsServiceRequest {
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
    }
}

fn logs_with_attributes() -> ExportLogsServiceRequest {
    ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: Some(Resource {
                attributes: vec![kv("service.name", "orders"), kv("deployment.environment", "production")],
                ..Default::default()
            }),
            scope_logs: vec![ScopeLogs {
                log_records: (0..1_000)
                    .map(|i| OtlpLogRecord {
                        time_unix_nano: 1_790_000_000_000_000_000 + i,
                        observed_time_unix_nano: 1_790_000_000_000_000_000 + i,
                        severity_number: 9,
                        severity_text: "INFO".into(),
                        body: Some(AnyValue { value: Some(Value::StringValue(format!("order {i} accepted"))) }),
                        attributes: vec![
                            kv("http.request.method", "POST"),
                            kv("http.route", "/orders/:id"),
                            kv("http.request.header.authorization", "Bearer synthetic-token"),
                        ],
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn ingest(c: &mut Criterion) {
    let input = traces();
    c.bench_function("ingest/sanitize_1000_traces", |b| {
        b.iter_batched(|| input.clone(), |mut payload| sanitize_traces(black_box(&mut payload)), BatchSize::SmallInput)
    });
    let attributed = traces_with_attributes();
    c.bench_function("ingest/sanitize_1000_traces_with_attributes", |b| {
        b.iter_batched(
            || attributed.clone(),
            |mut payload| sanitize_traces(black_box(&mut payload)),
            BatchSize::SmallInput,
        )
    });

    let metrics = metrics_with_attributes();
    c.bench_function("ingest/sanitize_1000_metric_points_with_attributes", |b| {
        b.iter_batched(
            || metrics.clone(),
            |mut payload| sanitize_metrics(black_box(&mut payload), &mut |_| true),
            BatchSize::SmallInput,
        )
    });

    let histograms = histograms_with_attributes();
    c.bench_function("ingest/sanitize_256_histograms_9_buckets", |b| {
        b.iter_batched(
            || histograms.clone(),
            |mut payload| sanitize_metrics(black_box(&mut payload), &mut |_| true),
            BatchSize::SmallInput,
        )
    });

    let logs = logs_with_attributes();
    c.bench_function("ingest/sanitize_1000_logs_with_attributes", |b| {
        b.iter_batched(|| logs.clone(), |mut payload| sanitize_logs(black_box(&mut payload)), BatchSize::SmallInput)
    });
}

criterion_group!(benches, ingest);
criterion_main!(benches);
