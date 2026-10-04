//! Server-side sanitisation of OTLP payloads before they are written to the
//! broker: redaction of sensitive attributes, size limits, ID validation.

use controlhub_domain::model::limits;
use controlhub_domain::redaction::{
    AttributeDecision, REDACTED, classify_key, mask_bearer, sanitize_attribute, sanitize_value, truncate,
};
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
    /// Attributes dropped because an item exceeded `MAX_ATTRIBUTES_PER_ITEM`.
    pub truncated_attrs: u64,
}

fn redacted_value() -> Option<AnyValue> {
    Some(AnyValue { value: Some(Value::StringValue(REDACTED.to_owned())) })
}

pub fn sanitize_attrs(attrs: &mut Vec<KeyValue>, stats: &mut SanitizeStats, depth: u32) {
    attrs.retain_mut(|kv| {
        // Classify the full key: a sensitive fragment past the length limit must still match.
        let decision = classify_key(&kv.key);
        if kv.key.len() > limits::MAX_ATTRIBUTE_KEY_LEN {
            kv.key = truncate(&kv.key, limits::MAX_ATTRIBUTE_KEY_LEN).to_owned();
        }
        match decision {
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
                if let Some(AnyValue { value: Some(Value::StringValue(s)) }) = kv.value.as_mut() {
                    let (clean, removed_query) = sanitize_attribute(&kv.key, s);
                    *s = clean;
                    if removed_query {
                        stats.redacted += 1;
                    }
                } else if let Some(v) = kv.value.as_mut() {
                    sanitize_any(v, stats, depth);
                }
                true
            }
        }
    });
    if attrs.len() > limits::MAX_ATTRIBUTES_PER_ITEM {
        stats.truncated_attrs += (attrs.len() - limits::MAX_ATTRIBUTES_PER_ITEM) as u64;
        attrs.truncate(limits::MAX_ATTRIBUTES_PER_ITEM);
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

    use opentelemetry_proto::tonic::common::v1::{ArrayValue, InstrumentationScope, KeyValueList};
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use opentelemetry_proto::tonic::trace::v1::{Status, span};

    fn sv(v: &str) -> Option<AnyValue> {
        Some(AnyValue { value: Some(Value::StringValue(v.into())) })
    }

    fn kv_any(k: &str, v: Value) -> KeyValue {
        KeyValue { key: k.into(), value: Some(AnyValue { value: Some(v) }), ..Default::default() }
    }

    fn string_of(v: &Option<AnyValue>) -> Option<&str> {
        match v.as_ref()?.value.as_ref()? {
            Value::StringValue(s) => Some(s),
            _ => None,
        }
    }

    fn attrs_after(mut attrs: Vec<KeyValue>) -> (Vec<KeyValue>, SanitizeStats) {
        let mut st = SanitizeStats::default();
        sanitize_attrs(&mut attrs, &mut st, 0);
        (attrs, st)
    }

    fn traces(spans: Vec<Span>) -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans { spans, ..Default::default() }],
                ..Default::default()
            }],
        }
    }

    fn logs(records: Vec<LogRecord>) -> ExportLogsServiceRequest {
        ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs { log_records: records, ..Default::default() }],
                ..Default::default()
            }],
        }
    }

    fn log(time: u64) -> LogRecord {
        LogRecord { time_unix_nano: time, ..Default::default() }
    }

    #[test]
    fn sensitive_keys_are_redacted_case_insensitively() {
        let (attrs, st) = attrs_after(vec![
            kv("HTTP.Request.Header.Authorization", "Bearer abc"),
            kv("http.request.header.cookie", "sid=1"),
            kv("User.PASSWORD", "hunter2"),
            kv("X-Api-Key", "k"),
            kv("http.route", "/x"),
        ]);
        assert_eq!(st.redacted, 4);
        assert_eq!(attrs.len(), 5);
        for a in &attrs[..4] {
            assert_eq!(string_of(&a.value), Some(REDACTED), "{}", a.key);
        }
        assert_eq!(string_of(&attrs[4].value), Some("/x"));
    }

    #[test]
    fn redaction_replaces_non_string_values_too() {
        let (attrs, st) = attrs_after(vec![
            kv_any("api_key", Value::IntValue(12345)),
            kv_any("session", Value::BytesValue(vec![1, 2, 3])),
            kv_any("credentials", Value::KvlistValue(KeyValueList { values: vec![kv("user", "u")] })),
        ]);
        assert_eq!(st.redacted, 3);
        assert!(attrs.iter().all(|a| string_of(&a.value) == Some(REDACTED)));
    }

    #[test]
    fn bodies_and_statements_are_dropped_and_counted() {
        let (attrs, st) = attrs_after(vec![
            kv("http.request.body", "{\"card\":\"4111\"}"),
            kv("HTTP.RESPONSE.BODY", "x"),
            kv("db.query.text", "SELECT 1"),
            kv("db.statement", "SELECT 2"),
            kv("db.system", "postgresql"),
        ]);
        assert_eq!(st.redacted, 4);
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].key, "db.system");
    }

    #[test]
    fn sensitive_fragment_beyond_key_limit_is_still_redacted() {
        // Regression: keys were truncated *before* classification, so a fragment past
        // MAX_ATTRIBUTE_KEY_LEN escaped redaction.
        let key = format!("{}.password", "a".repeat(limits::MAX_ATTRIBUTE_KEY_LEN));
        let (attrs, st) = attrs_after(vec![kv(&key, "hunter2")]);
        assert_eq!(st.redacted, 1);
        assert_eq!(attrs[0].key.len(), limits::MAX_ATTRIBUTE_KEY_LEN);
        assert_eq!(string_of(&attrs[0].value), Some(REDACTED));
    }

    #[test]
    fn long_keys_are_truncated_on_char_boundaries() {
        let key = "ж".repeat(limits::MAX_ATTRIBUTE_KEY_LEN);
        let (attrs, _) = attrs_after(vec![kv(&key, "v")]);
        assert!(attrs[0].key.len() <= limits::MAX_ATTRIBUTE_KEY_LEN);
        assert!(attrs[0].key.chars().all(|c| c == 'ж'));
    }

    #[test]
    fn values_are_truncated_and_bearer_masked() {
        let huge = "v".repeat(limits::MAX_ATTRIBUTE_VALUE_LEN * 2);
        let (attrs, _) = attrs_after(vec![kv("big", &huge), kv("msg", "call failed: Bearer abc.def")]);
        assert_eq!(string_of(&attrs[0].value).map(str::len), Some(limits::MAX_ATTRIBUTE_VALUE_LEN));
        assert_eq!(string_of(&attrs[1].value), Some("call failed: Bearer [REDACTED]"));
    }

    #[test]
    fn url_query_strings_are_redacted_and_counted() {
        let (attrs, stats) = attrs_after(vec![
            kv("url.full", "http://orders:7004/orders/9?token=abc"),
            kv("url.query", "email=a@b.c"),
            kv("url.path", "/orders/9"),
        ]);
        assert_eq!(string_of(&attrs[0].value), Some("http://orders:7004/orders/9?[REDACTED]"));
        assert_eq!(string_of(&attrs[1].value), Some(REDACTED));
        assert_eq!(string_of(&attrs[2].value), Some("/orders/9"));
        assert_eq!(stats.redacted, 2);
    }

    #[test]
    fn attribute_count_limit_counts_every_dropped_attribute() {
        let many: Vec<_> = (0..limits::MAX_ATTRIBUTES_PER_ITEM + 10).map(|i| kv(&format!("k{i}"), "v")).collect();
        let (attrs, st) = attrs_after(many);
        assert_eq!(attrs.len(), limits::MAX_ATTRIBUTES_PER_ITEM);
        assert_eq!(st.truncated_attrs, 10);
        let (_, st) = attrs_after(vec![kv("a", "b")]);
        assert_eq!(st.truncated_attrs, 0);
    }

    #[test]
    fn nested_kvlists_and_arrays_are_redacted() {
        let inner = KeyValueList { values: vec![kv("password", "p"), kv("db.statement", "q"), kv("ok", "Bearer t")] };
        let arr = ArrayValue {
            values: vec![
                AnyValue { value: Some(Value::KvlistValue(inner.clone())) },
                sv("Bearer t2").unwrap_or_default(),
            ],
        };
        let (attrs, st) =
            attrs_after(vec![kv_any("config", Value::KvlistValue(inner)), kv_any("list", Value::ArrayValue(arr))]);
        assert_eq!(st.redacted, 4, "two nested sensitive keys at two places");
        let dump = format!("{attrs:?}");
        assert!(!dump.contains("\"p\""), "{dump}");
        assert!(!dump.contains("\"q\""), "{dump}");
        assert!(!dump.contains("Bearer t"), "{dump}");
    }

    #[test]
    fn deep_nesting_is_cut_off() {
        let mut v = Value::StringValue("leaf password=x".into());
        for _ in 0..10 {
            v = Value::ArrayValue(ArrayValue { values: vec![AnyValue { value: Some(v) }] });
        }
        let (attrs, _) = attrs_after(vec![kv_any("deep", v)]);
        let dump = format!("{attrs:?}");
        assert!(dump.contains("[nested value omitted]"));
        assert!(!dump.contains("leaf"));

        let mut kvl = Value::StringValue("leaf".into());
        for _ in 0..10 {
            kvl = Value::KvlistValue(KeyValueList { values: vec![kv_any("k", kvl)] });
        }
        let (attrs, _) = attrs_after(vec![kv_any("deep", kvl)]);
        assert!(format!("{attrs:?}").contains("[nested value omitted]"));
    }

    #[test]
    fn arrays_and_bytes_are_bounded() {
        let arr =
            ArrayValue { values: (0..MAX_ARRAY_LEN + 50).map(|i| sv(&i.to_string()).unwrap_or_default()).collect() };
        let (attrs, _) = attrs_after(vec![
            kv_any("arr", Value::ArrayValue(arr)),
            kv_any("small", Value::BytesValue(vec![7; 256])),
            kv_any("big", Value::BytesValue(vec![7; 257])),
            kv_any("n", Value::IntValue(-1)),
            kv_any("b", Value::BoolValue(true)),
            kv_any("d", Value::DoubleValue(1.5)),
        ]);
        let Some(Value::ArrayValue(a)) = attrs[0].value.as_ref().and_then(|v| v.value.clone()) else { panic!() };
        assert_eq!(a.values.len(), MAX_ARRAY_LEN);
        assert!(
            matches!(attrs[1].value.as_ref().and_then(|v| v.value.as_ref()), Some(Value::BytesValue(b)) if b.len() == 256)
        );
        assert_eq!(string_of(&attrs[2].value), Some("[bytes omitted]"));
        assert!(matches!(attrs[3].value.as_ref().and_then(|v| v.value.as_ref()), Some(Value::IntValue(-1))));
        assert!(matches!(attrs[4].value.as_ref().and_then(|v| v.value.as_ref()), Some(Value::BoolValue(true))));
        assert!(matches!(attrs[5].value.as_ref().and_then(|v| v.value.as_ref()), Some(Value::DoubleValue(_))));
    }

    #[test]
    fn missing_values_are_tolerated() {
        let (attrs, _) = attrs_after(vec![
            KeyValue { key: "k".into(), value: None, ..Default::default() },
            KeyValue { key: "e".into(), value: Some(AnyValue { value: None }), ..Default::default() },
        ]);
        assert_eq!(attrs.len(), 2);
    }

    #[test]
    fn span_validation_rejects_bad_ids_and_times() {
        let good = || span([1; 16], [2; 8]);
        let mut cases = vec![good()];
        let mut s = good();
        s.trace_id = vec![1; 15];
        cases.push(s);
        let mut s = good();
        s.span_id = vec![0; 8];
        cases.push(s);
        let mut s = good();
        s.span_id = vec![];
        cases.push(s);
        let mut s = good();
        s.parent_span_id = vec![3; 4];
        cases.push(s);
        let mut s = good();
        s.end_time_unix_nano = s.start_time_unix_nano - 1;
        cases.push(s);
        let mut s = good();
        s.start_time_unix_nano = 0;
        s.end_time_unix_nano = 0;
        cases.push(s);
        let mut s = good();
        s.parent_span_id = vec![3; 8];
        s.end_time_unix_nano = s.start_time_unix_nano;
        cases.push(s);
        let mut req = traces(cases);
        let st = sanitize_traces(&mut req);
        assert_eq!(st.accepted, 2);
        assert_eq!(st.rejected, 6);
        assert_eq!(req.resource_spans[0].scope_spans[0].spans.len(), 2);
    }

    #[test]
    fn span_budget_rejects_the_overflow() {
        let spans = (0..MAX_SPANS_PER_REQUEST + 3)
            .map(|_| Span {
                trace_id: vec![1; 16],
                span_id: vec![2; 8],
                start_time_unix_nano: 1,
                end_time_unix_nano: 2,
                ..Default::default()
            })
            .collect();
        let mut req = traces(spans);
        let st = sanitize_traces(&mut req);
        assert_eq!(st.accepted, MAX_SPANS_PER_REQUEST as u64);
        assert_eq!(st.rejected, 3);
    }

    #[test]
    fn every_span_attribute_location_is_sanitized() {
        let mut s = span([1; 16], [2; 8]);
        s.name = "n".repeat(MAX_NAME_LEN + 10);
        s.events = vec![span::Event { attributes: vec![kv("cookie", "c1")], ..Default::default() }];
        s.links = vec![span::Link { attributes: vec![kv("token", "t1")], ..Default::default() }];
        s.status = Some(Status { message: "denied for Bearer zzz".into(), code: 2 });
        let mut req = traces(vec![s]);
        req.resource_spans[0].resource = Some(Resource { attributes: vec![kv("secret", "r1")], ..Default::default() });
        req.resource_spans[0].scope_spans[0].scope =
            Some(InstrumentationScope { attributes: vec![kv("passwd", "s1")], ..Default::default() });
        let st = sanitize_traces(&mut req);
        // resource, scope, event, link + authorization on the span; db.statement dropped.
        assert_eq!(st.redacted, 6);
        let dump = format!("{req:?}");
        for leaked in ["\"c1\"", "\"t1\"", "\"r1\"", "\"s1\"", "zzz", "secret-token"] {
            assert!(!dump.contains(leaked), "{leaked} leaked");
        }
        let out = &req.resource_spans[0].scope_spans[0].spans[0];
        assert_eq!(out.name.len(), MAX_NAME_LEN);
        assert_eq!(out.status.as_ref().map(|s| s.message.as_str()), Some("denied for Bearer [REDACTED]"));
    }

    #[test]
    fn empty_requests_are_fine() {
        assert_eq!(sanitize_traces(&mut ExportTraceServiceRequest::default()), SanitizeStats::default());
        assert_eq!(sanitize_logs(&mut ExportLogsServiceRequest::default()), SanitizeStats::default());
    }

    #[test]
    fn log_validation() {
        let mut observed_only = log(0);
        observed_only.observed_time_unix_nano = 5;
        let mut bad_trace = log(1);
        bad_trace.trace_id = vec![1; 8];
        let mut bad_span = log(1);
        bad_span.span_id = vec![1; 16];
        let mut good_ids = log(1);
        good_ids.trace_id = vec![1; 16];
        good_ids.span_id = vec![1; 8];
        let mut req = logs(vec![log(1), observed_only, log(0), bad_trace, bad_span, good_ids]);
        let st = sanitize_logs(&mut req);
        assert_eq!(st.accepted, 3);
        assert_eq!(st.rejected, 3);
    }

    #[test]
    fn log_budget_rejects_the_overflow() {
        let mut req = logs((0..MAX_LOGS_PER_REQUEST + 2).map(|_| log(1)).collect());
        let st = sanitize_logs(&mut req);
        assert_eq!(st.accepted, MAX_LOGS_PER_REQUEST as u64);
        assert_eq!(st.rejected, 2);
    }

    #[test]
    fn log_bodies_are_truncated_masked_and_structured_bodies_redacted() {
        let mut long = log(1);
        long.body = sv(&format!("Bearer abc {}", "x".repeat(limits::MAX_LOG_BODY_LEN * 2)));
        let mut structured = log(1);
        structured.body = Some(AnyValue {
            value: Some(Value::KvlistValue(KeyValueList {
                values: vec![kv("authorization", "Basic dXNlcg=="), kv("msg", "ok")],
            })),
        });
        let mut with_attrs = log(1);
        with_attrs.attributes = vec![kv("http.request.body", "raw"), kv("api-key", "k")];
        with_attrs.body = None;
        let mut req = logs(vec![long, structured, with_attrs]);
        req.resource_logs[0].resource = Some(Resource { attributes: vec![kv("token", "rt")], ..Default::default() });
        req.resource_logs[0].scope_logs[0].scope =
            Some(InstrumentationScope { attributes: vec![kv("cookie", "sc")], ..Default::default() });
        let st = sanitize_logs(&mut req);
        assert_eq!(st.accepted, 3);
        assert_eq!(st.redacted, 5);
        let recs = &req.resource_logs[0].scope_logs[0].log_records;
        let body = string_of(&recs[0].body).unwrap();
        assert!(body.starts_with("Bearer [REDACTED] x"));
        // Truncation happens before masking, so a mask may add a few bytes.
        assert!(body.len() <= limits::MAX_LOG_BODY_LEN + REDACTED.len());
        let dump = format!("{req:?}");
        for leaked in ["dXNlcg==", "\"raw\"", "\"k\"", "\"rt\"", "\"sc\"", "abc"] {
            assert!(!dump.contains(leaked), "{leaked} leaked");
        }
        assert_eq!(recs[2].attributes.len(), 1);
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        const FRAGMENTS: &[&str] = &[
            "authorization",
            "cookie",
            "password",
            "passwd",
            "secret",
            "token",
            "api_key",
            "apikey",
            "api-key",
            "private_key",
            "credential",
            "session",
            "x-csrf",
        ];
        const MARKER: &str = "SECRETVALUE";

        fn key() -> impl Strategy<Value = String> {
            prop_oneof![
                "[a-z.]{1,16}",
                ("[a-zA-Z._-]{0,8}", prop::sample::select(FRAGMENTS), "[a-zA-Z._-]{0,8}", any::<bool>(), 0usize..200)
                    .prop_map(|(pre, f, suf, upper, pad)| {
                        let f = if upper { f.to_ascii_uppercase() } else { f.to_owned() };
                        format!("{}{pre}{f}{suf}", "p".repeat(pad))
                    }),
                prop::sample::select(vec!["http.request.body", "DB.STATEMENT", "db.query.text", "http.response.body"])
                    .prop_map(str::to_owned),
            ]
        }

        fn value() -> impl Strategy<Value = AnyValue> {
            let leaf = prop_oneof![
                Just(Value::StringValue(MARKER.into())),
                ".{0,32}".prop_map(Value::StringValue),
                any::<i64>().prop_map(Value::IntValue),
                any::<bool>().prop_map(Value::BoolValue),
                prop::collection::vec(any::<u8>(), 0..300).prop_map(Value::BytesValue),
            ]
            .prop_map(|v| AnyValue { value: Some(v) });
            leaf.prop_recursive(6, 64, 8, |inner| {
                prop_oneof![
                    prop::collection::vec(inner.clone(), 0..8)
                        .prop_map(|values| AnyValue { value: Some(Value::ArrayValue(ArrayValue { values })) }),
                    prop::collection::vec((key(), inner), 0..8).prop_map(|kvs| AnyValue {
                        value: Some(Value::KvlistValue(KeyValueList {
                            values: kvs
                                .into_iter()
                                .map(|(key, v)| KeyValue { key, value: Some(v), ..Default::default() })
                                .collect(),
                        })),
                    }),
                ]
            })
        }

        /// Walks the sanitised tree and checks the invariants at every level.
        fn check(attrs: &[KeyValue]) -> Result<(), TestCaseError> {
            prop_assert!(attrs.len() <= limits::MAX_ATTRIBUTES_PER_ITEM);
            for kv in attrs {
                prop_assert!(kv.key.len() <= limits::MAX_ATTRIBUTE_KEY_LEN);
                let lower = kv.key.to_ascii_lowercase();
                prop_assert!(
                    !["http.request.body", "http.response.body", "db.statement", "db.query.text"]
                        .contains(&lower.as_str()),
                    "dropped key survived: {}",
                    kv.key
                );
                if FRAGMENTS.iter().any(|f| lower.contains(f)) {
                    prop_assert_eq!(string_of(&kv.value), Some(REDACTED));
                }
                if let Some(v) = &kv.value {
                    check_value(v)?;
                }
            }
            Ok(())
        }

        fn check_value(v: &AnyValue) -> Result<(), TestCaseError> {
            match &v.value {
                Some(Value::StringValue(s)) => {
                    prop_assert!(s.len() <= limits::MAX_ATTRIBUTE_VALUE_LEN + REDACTED.len())
                }
                Some(Value::BytesValue(b)) => prop_assert!(b.len() <= 256),
                Some(Value::ArrayValue(a)) => {
                    prop_assert!(a.values.len() <= MAX_ARRAY_LEN);
                    for x in &a.values {
                        check_value(x)?;
                    }
                }
                Some(Value::KvlistValue(kv)) => check(&kv.values)?,
                _ => {}
            }
            Ok(())
        }

        /// No value under a sensitive or dropped key survives anywhere in the tree.
        fn no_marker_under_sensitive_keys(attrs: &[KeyValue]) -> bool {
            attrs.iter().all(|kv| {
                let sensitive = !matches!(controlhub_domain::redaction::classify_key(&kv.key), AttributeDecision::Keep);
                let dump = format!("{:?}", kv.value);
                !(sensitive && dump.contains(MARKER))
                    && match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
                        Some(Value::KvlistValue(l)) => no_marker_under_sensitive_keys(&l.values),
                        _ => true,
                    }
            })
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]

            #[test]
            fn sanitized_attributes_never_contain_secret_keys(
                kvs in prop::collection::vec((key(), value()), 0..80)
            ) {
                let attrs: Vec<KeyValue> =
                    kvs.into_iter().map(|(key, v)| KeyValue { key, value: Some(v), ..Default::default() }).collect();
                let before = attrs.len() as u64;
                let (out, st) = attrs_after(attrs);
                check(&out)?;
                prop_assert!(no_marker_under_sensitive_keys(&out));
                // Top-level accounting: nothing disappears without a counter.
                let dropped_top = before - out.len() as u64;
                prop_assert!(dropped_top <= st.redacted + st.truncated_attrs);
            }

            #[test]
            fn secret_value_never_survives_under_a_sensitive_key_of_any_length(
                pad in 0usize..300,
                upper in any::<bool>(),
                fragment in prop::sample::select(FRAGMENTS),
                v in value(),
            ) {
                let f = if upper { fragment.to_ascii_uppercase() } else { fragment.to_owned() };
                let key = format!("{}{f}", "x".repeat(pad));
                let attrs = vec![KeyValue { key, value: Some(v), ..Default::default() }];
                let (out, st) = attrs_after(attrs);
                prop_assert_eq!(st.redacted, 1);
                let dump = format!("{out:?}");
                prop_assert!(!dump.contains(MARKER));
            }

            #[test]
            fn sanitize_is_idempotent(kvs in prop::collection::vec((key(), value()), 0..20)) {
                let attrs: Vec<KeyValue> =
                    kvs.into_iter().map(|(key, v)| KeyValue { key, value: Some(v), ..Default::default() }).collect();
                let (once, _) = attrs_after(attrs);
                let (twice, _) = attrs_after(once.clone());
                prop_assert_eq!(once, twice);
            }

            #[test]
            fn span_accounting_is_complete(
                ids in prop::collection::vec((prop::collection::vec(any::<u8>(), 15..17), prop::collection::vec(0u8..2, 7..9), 0u64..3, 0u64..3), 0..40)
            ) {
                let n = ids.len() as u64;
                let spans = ids
                    .into_iter()
                    .map(|(t, s, start, end)| Span { trace_id: t, span_id: s, start_time_unix_nano: start, end_time_unix_nano: end, ..Default::default() })
                    .collect();
                let mut req = traces(spans);
                let st = sanitize_traces(&mut req);
                prop_assert_eq!(st.accepted + st.rejected, n);
                prop_assert_eq!(req.resource_spans[0].scope_spans[0].spans.len() as u64, st.accepted);
                for s in &req.resource_spans[0].scope_spans[0].spans {
                    prop_assert!(valid_id(&s.trace_id, 16) && valid_id(&s.span_id, 8));
                    prop_assert!(s.start_time_unix_nano > 0 && s.end_time_unix_nano >= s.start_time_unix_nano);
                }
            }
        }
    }
}
