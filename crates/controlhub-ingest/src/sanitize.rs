//! Server-side sanitisation of OTLP payloads before they are written to the
//! broker: redaction of sensitive attributes, size limits, ID validation.

use controlhub_domain::model::limits;
use controlhub_domain::redaction::{AttributeDecision, REDACTED, classify_key, mask_bearer, sanitize_value, truncate};
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value};

pub const MAX_SPANS_PER_REQUEST: usize = 20_000;
pub const MAX_LOGS_PER_REQUEST: usize = 20_000;
const MAX_NAME_LEN: usize = 1_024;
const MAX_DEPTH: u32 = 4;
const MAX_ARRAY_LEN: usize = 128;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SanitizeStats {
    pub accepted: u64,
    pub rejected: u64,
    pub redacted: u64,
    pub truncated_attrs: u64,
}

fn redacted_value() -> Option<AnyValue> {
    Some(AnyValue { value: Some(Value::StringValue(REDACTED.to_owned())) })
}

pub fn sanitize_attrs(attrs: &mut Vec<KeyValue>, stats: &mut SanitizeStats, depth: u32) {
    attrs.retain_mut(|kv| {
        if kv.key.len() > limits::MAX_ATTRIBUTE_KEY_LEN {
            kv.key = truncate(&kv.key, limits::MAX_ATTRIBUTE_KEY_LEN).to_owned();
        }
        match classify_key(&kv.key) {
            AttributeDecision::Drop => {
                stats.redacted += 1;
                false
            }
            AttributeDecision::Redact => {
                stats.redacted += 1;
                kv.value = redacted_value();
                true
            }
            AttributeDecision::Keep => {
                if let Some(v) = kv.value.as_mut() {
                    sanitize_any(v, stats, depth);
                }
                true
            }
        }
    });
    if attrs.len() > limits::MAX_ATTRIBUTES_PER_ITEM {
        attrs.truncate(limits::MAX_ATTRIBUTES_PER_ITEM);
        stats.truncated_attrs += 1;
    }
}

fn sanitize_any(v: &mut AnyValue, stats: &mut SanitizeStats, depth: u32) {
    let Some(inner) = v.value.as_mut() else { return };
    match inner {
        Value::StringValue(s) => *s = sanitize_value(s),
        Value::BytesValue(b) => {
            if b.len() > 256 {
                *inner = Value::StringValue("[bytes omitted]".into());
            }
        }
        Value::ArrayValue(arr) => {
            if depth >= MAX_DEPTH {
                *inner = Value::StringValue("[nested value omitted]".into());
                return;
            }
            arr.values.truncate(MAX_ARRAY_LEN);
            for x in &mut arr.values {
                sanitize_any(x, stats, depth + 1);
            }
        }
        Value::KvlistValue(kv) => {
            if depth >= MAX_DEPTH {
                *inner = Value::StringValue("[nested value omitted]".into());
                return;
            }
            sanitize_attrs(&mut kv.values, stats, depth + 1);
        }
        Value::BoolValue(_) | Value::IntValue(_) | Value::DoubleValue(_) => {}
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

fn valid_id(bytes: &[u8], len: usize) -> bool {
    bytes.len() == len && bytes.iter().any(|b| *b != 0)
}

/// Returns stats; invalid spans (bad IDs, end < start) are removed and counted as rejected.
pub fn sanitize_traces(req: &mut ExportTraceServiceRequest) -> SanitizeStats {
    let mut st = SanitizeStats::default();
    let mut budget = MAX_SPANS_PER_REQUEST;
    for rs in &mut req.resource_spans {
        if let Some(r) = rs.resource.as_mut() {
            sanitize_attrs(&mut r.attributes, &mut st, 0);
        }
        for ss in &mut rs.scope_spans {
            if let Some(scope) = ss.scope.as_mut() {
                sanitize_attrs(&mut scope.attributes, &mut st, 0);
            }
            ss.spans.retain_mut(|span| {
                let ok = budget > 0
                    && valid_id(&span.trace_id, 16)
                    && valid_id(&span.span_id, 8)
                    && (span.parent_span_id.is_empty() || span.parent_span_id.len() == 8)
                    && span.end_time_unix_nano >= span.start_time_unix_nano
                    && span.start_time_unix_nano > 0;
                if !ok {
                    st.rejected += 1;
                    return false;
                }
                budget -= 1;
                if span.name.len() > MAX_NAME_LEN {
                    span.name = truncate(&span.name, MAX_NAME_LEN).to_owned();
                }
                sanitize_attrs(&mut span.attributes, &mut st, 0);
                for ev in &mut span.events {
                    sanitize_attrs(&mut ev.attributes, &mut st, 0);
                }
                for link in &mut span.links {
                    sanitize_attrs(&mut link.attributes, &mut st, 0);
                }
                if let Some(status) = span.status.as_mut() {
                    status.message = sanitize_value(&status.message);
                }
                st.accepted += 1;
                true
            });
        }
    }
    st
}

pub fn sanitize_logs(req: &mut ExportLogsServiceRequest) -> SanitizeStats {
    let mut st = SanitizeStats::default();
    let mut budget = MAX_LOGS_PER_REQUEST;
    for rl in &mut req.resource_logs {
        if let Some(r) = rl.resource.as_mut() {
            sanitize_attrs(&mut r.attributes, &mut st, 0);
        }
        for sl in &mut rl.scope_logs {
            if let Some(scope) = sl.scope.as_mut() {
                sanitize_attrs(&mut scope.attributes, &mut st, 0);
            }
            sl.log_records.retain_mut(|lr| {
                let ids_ok = (lr.trace_id.is_empty() || lr.trace_id.len() == 16)
                    && (lr.span_id.is_empty() || lr.span_id.len() == 8);
                if budget == 0 || !ids_ok || (lr.time_unix_nano == 0 && lr.observed_time_unix_nano == 0) {
                    st.rejected += 1;
                    return false;
                }
                budget -= 1;
                if let Some(body) = lr.body.as_mut() {
                    match body.value.as_mut() {
                        Some(Value::StringValue(s)) => {
                            *s = mask_bearer(truncate(s, limits::MAX_LOG_BODY_LEN));
                        }
                        _ => sanitize_any(body, &mut st, 0),
                    }
                }
                sanitize_attrs(&mut lr.attributes, &mut st, 0);
                st.accepted += 1;
                true
            });
        }
    }
    st
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use opentelemetry_proto::tonic::resource::v1::Resource;
    use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};

    fn kv(k: &str, v: &str) -> KeyValue {
        KeyValue {
            key: k.into(),
            value: Some(AnyValue { value: Some(Value::StringValue(v.into())) }),
            ..Default::default()
        }
    }

    fn span(trace: [u8; 16], id: [u8; 8]) -> Span {
        Span {
            trace_id: trace.to_vec(),
            span_id: id.to_vec(),
            name: "GET /".into(),
            start_time_unix_nano: 10,
            end_time_unix_nano: 20,
            attributes: vec![
                kv("http.request.header.authorization", "Bearer secret-token"),
                kv("db.statement", "SELECT * FROM users WHERE password='x'"),
                kv("http.route", "/orders"),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn redacts_and_rejects() {
        let mut req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource { attributes: vec![kv("service.name", "api")], ..Default::default() }),
                scope_spans: vec![ScopeSpans {
                    spans: vec![span([1; 16], [2; 8]), span([0; 16], [2; 8])],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let st = sanitize_traces(&mut req);
        assert_eq!(st.accepted, 1);
        assert_eq!(st.rejected, 1);
        let attrs = &req.resource_spans[0].scope_spans[0].spans[0].attributes;
        assert_eq!(attrs.len(), 2, "db.statement dropped");
        let auth = attrs.iter().find(|a| a.key.contains("authorization")).unwrap();
        assert_eq!(auth.value, redacted_value());
        let encoded = format!("{req:?}");
        assert!(!encoded.contains("secret-token"));
        assert!(!encoded.contains("password='x'"));
    }
}
