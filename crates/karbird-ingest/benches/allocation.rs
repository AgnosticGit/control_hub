#![allow(clippy::expect_used)]

use divan::{AllocProfiler, black_box};
use karbird_ingest::sanitize::sanitize_traces;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value};
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};

#[global_allocator]
static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
}

fn fixture() -> ExportTraceServiceRequest {
    ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans: (0..1_000)
                    .map(|i| Span {
                        trace_id: vec![1; 16],
                        span_id: (i as u64 + 1).to_be_bytes().to_vec(),
                        name: format!("GET /orders/{i}"),
                        kind: 2,
                        start_time_unix_nano: 1_790_000_000_000_000_000 + i as u64,
                        end_time_unix_nano: 1_790_000_001_000_000_000 + i as u64,
                        attributes: vec![
                            KeyValue {
                                key: "http.request.header.authorization".into(),
                                value: Some(AnyValue { value: Some(Value::StringValue("Bearer synthetic".into())) }),
                                ..Default::default()
                            },
                            KeyValue {
                                key: "http.route".into(),
                                value: Some(AnyValue { value: Some(Value::StringValue("/orders/:id".into())) }),
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
    }
}

#[divan::bench]
fn sanitize_1000_traces() -> usize {
    let mut req = fixture();
    let stats = sanitize_traces(black_box(&mut req));
    stats.redacted as usize
}
