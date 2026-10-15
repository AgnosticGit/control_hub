//! Server-side sanitisation of OTLP payloads before they are written to the
//! broker: redaction of sensitive attributes, size limits, ID validation.

use karbird_domain::model::limits;
use karbird_domain::redaction::{
    AttributeDecision, REDACTED, classify_key, sanitize_attribute_in_place, sanitize_value_in_place,
    sanitize_value_in_place_with_limit, truncate,
};
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value::Value};
use opentelemetry_proto::tonic::metrics::v1::{NumberDataPoint, metric::Data, number_data_point};

pub const MAX_SPANS_PER_REQUEST: usize = 20_000;
pub const MAX_LOGS_PER_REQUEST: usize = 20_000;
pub const MAX_POINTS_PER_REQUEST: usize = 20_000;
const MAX_METRIC_NAME_LEN: usize = 255;
const MAX_UNIT_LEN: usize = 63;
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
                    let removed_query = sanitize_attribute_in_place(&kv.key, s);
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
        Value::StringValue(s) => sanitize_value_in_place(s),
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
                    sanitize_value_in_place(&mut status.message);
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
                            sanitize_value_in_place_with_limit(s, limits::MAX_LOG_BODY_LEN);
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MetricStats {
    pub accepted: u64,
    /// Invalid points, unsupported metric types (exponential histograms, summaries) and the per-request overflow.
    pub rejected: u64,
    /// Points of new series refused by the per-environment series limit.
    pub over_limit: u64,
    pub redacted: u64,
    pub truncated_attrs: u64,
}

impl MetricStats {
    pub fn refused(&self) -> u64 {
        self.rejected + self.over_limit
    }
}

/// Dotted identifier as used by OpenTelemetry semantic conventions (`system.cpu.time`).
pub fn valid_metric_name(name: &str) -> bool {
    name.len() <= MAX_METRIC_NAME_LEN
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
}

fn hash_bytes(h: &mut u64, bytes: &[u8]) {
    for b in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0100_0000_01b3);
    }
}

fn sorted_attrs(attrs: &[KeyValue]) -> Vec<&KeyValue> {
    let mut sorted = attrs.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    sorted
}

fn hash_attribute(h: &mut u64, kv: &KeyValue) {
    hash_bytes(h, kv.key.as_bytes());
    match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
        Some(Value::StringValue(s)) => hash_bytes(h, s.as_bytes()),
        other => hash_bytes(h, format!("{other:?}").as_bytes()),
    }
}

fn hash_sorted_attrs(h: &mut u64, sorted: &[&KeyValue]) {
    for kv in sorted {
        hash_attribute(h, kv);
    }
}

fn hash_attrs(h: &mut u64, attrs: &[KeyValue]) {
    hash_sorted_attrs(h, &sorted_attrs(attrs));
}

fn series_key_with_sorted_resource(name: &str, resource: &[&KeyValue], point: &[KeyValue]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    hash_bytes(&mut h, name.as_bytes());
    hash_sorted_attrs(&mut h, resource);
    hash_bytes(&mut h, b"|");
    hash_attrs(&mut h, point);
    h
}

/// Identity of one time series: metric name, resource and point attributes (FNV-1a).
pub fn series_key(name: &str, resource: &[KeyValue], point: &[KeyValue]) -> u64 {
    series_key_with_sorted_resource(name, &sorted_attrs(resource), point)
}

fn valid_point(p: &NumberDataPoint) -> bool {
    p.time_unix_nano > 0
        && match p.value {
            Some(number_data_point::Value::AsDouble(v)) => v.is_finite(),
            Some(number_data_point::Value::AsInt(_)) => true,
            None => false,
        }
}

fn valid_histogram_point(p: &opentelemetry_proto::tonic::metrics::v1::HistogramDataPoint) -> bool {
    p.time_unix_nano > 0
        && !p.bucket_counts.is_empty()
        && p.bucket_counts.len() <= 65
        && p.bucket_counts.len() == p.explicit_bounds.len() + 1
        && p.explicit_bounds.iter().all(|b| b.is_finite())
        && p.explicit_bounds.windows(2).all(|pair| pair[0] < pair[1])
        && p.bucket_counts.iter().try_fold(0u64, |sum, count| sum.checked_add(*count)) == Some(p.count)
        && p.sum.is_none_or(f64::is_finite)
        && p.min.is_none_or(f64::is_finite)
        && p.max.is_none_or(f64::is_finite)
}

/// Keeps gauges, sums and explicit-boundary histograms; everything else is removed and counted. `admit` decides
/// whether a point's series fits the environment's series limit.
pub fn sanitize_metrics(req: &mut ExportMetricsServiceRequest, admit: &mut dyn FnMut(u64) -> bool) -> MetricStats {
    let mut st = MetricStats::default();
    let mut attr_stats = SanitizeStats::default();
    let mut budget = MAX_POINTS_PER_REQUEST;
    for rm in &mut req.resource_metrics {
        if let Some(r) = rm.resource.as_mut() {
            sanitize_attrs(&mut r.attributes, &mut attr_stats, 0);
        }
        let resource: Vec<KeyValue> = rm.resource.as_ref().map(|r| r.attributes.clone()).unwrap_or_default();
        // Resource attributes are shared by every point in this ResourceMetrics group.
        // Sort once here instead of allocating and sorting the same keys for each point.
        let sorted_resource = sorted_attrs(&resource);
        for sm in &mut rm.scope_metrics {
            if let Some(scope) = sm.scope.as_mut() {
                sanitize_attrs(&mut scope.attributes, &mut attr_stats, 0);
            }
            sm.metrics.retain_mut(|m| {
                let point_count = match m.data.as_ref() {
                    Some(Data::Gauge(g)) => g.data_points.len(),
                    Some(Data::Sum(s)) => s.data_points.len(),
                    Some(Data::Histogram(h)) => h.data_points.len(),
                    Some(Data::ExponentialHistogram(h)) => h.data_points.len(),
                    Some(Data::Summary(s)) => s.data_points.len(),
                    None => return false,
                };
                if !valid_metric_name(&m.name) {
                    st.rejected += point_count as u64;
                    return false;
                }
                m.description.clear();
                m.metadata.clear();
                if m.unit.len() > MAX_UNIT_LEN || !m.unit.bytes().all(|b| b.is_ascii_graphic()) {
                    m.unit.clear();
                }
                let name = m.name.clone();
                if let Some(Data::Histogram(h)) = m.data.as_mut() {
                    if !matches!(h.aggregation_temporality, 1 | 2) {
                        st.rejected += h.data_points.len() as u64;
                        return false;
                    }
                    h.data_points.retain_mut(|p| {
                        let cost = p.bucket_counts.len().saturating_add(2);
                        if !valid_histogram_point(p) || budget < cost {
                            st.rejected += 1;
                            return false;
                        }
                        sanitize_attrs(&mut p.attributes, &mut attr_stats, 0);
                        if !admit(series_key_with_sorted_resource(&name, &sorted_resource, &p.attributes)) {
                            st.over_limit += 1;
                            return false;
                        }
                        budget -= cost;
                        p.exemplars.clear();
                        st.accepted += 1;
                        true
                    });
                    return !h.data_points.is_empty();
                }
                let points = match m.data.as_mut() {
                    Some(Data::Gauge(g)) => &mut g.data_points,
                    Some(Data::Sum(s)) => &mut s.data_points,
                    Some(Data::Histogram(_)) => return false,
                    Some(Data::ExponentialHistogram(h)) => {
                        st.rejected += h.data_points.len() as u64;
                        return false;
                    }
                    Some(Data::Summary(s)) => {
                        st.rejected += s.data_points.len() as u64;
                        return false;
                    }
                    None => return false,
                };
                points.retain_mut(|p| {
                    if budget == 0 || !valid_point(p) {
                        st.rejected += 1;
                        return false;
                    }
                    sanitize_attrs(&mut p.attributes, &mut attr_stats, 0);
                    if !admit(series_key_with_sorted_resource(&name, &sorted_resource, &p.attributes)) {
                        st.over_limit += 1;
                        return false;
                    }
                    budget -= 1;
                    p.exemplars.clear();
                    st.accepted += 1;
                    true
                });
                !points.is_empty()
            });
        }
        rm.scope_metrics.retain(|sm| !sm.metrics.is_empty());
    }
    req.resource_metrics.retain(|rm| !rm.scope_metrics.is_empty());
    st.redacted = attr_stats.redacted;
    st.truncated_attrs = attr_stats.truncated_attrs;
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

    use opentelemetry_proto::tonic::metrics::v1::{
        Exemplar, Gauge, Histogram, HistogramDataPoint, Metric, ResourceMetrics, ScopeMetrics, Sum, Summary,
        SummaryDataPoint,
    };

    fn point(time: u64, v: f64, attrs: Vec<KeyValue>) -> NumberDataPoint {
        NumberDataPoint {
            attributes: attrs,
            time_unix_nano: time,
            value: Some(number_data_point::Value::AsDouble(v)),
            ..Default::default()
        }
    }

    fn gauge(name: &str, points: Vec<NumberDataPoint>) -> Metric {
        Metric { name: name.into(), data: Some(Data::Gauge(Gauge { data_points: points })), ..Default::default() }
    }

    fn metrics_req(resource: Vec<KeyValue>, metrics: Vec<Metric>) -> ExportMetricsServiceRequest {
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource { attributes: resource, ..Default::default() }),
                scope_metrics: vec![ScopeMetrics { metrics, ..Default::default() }],
                ..Default::default()
            }],
        }
    }

    fn admit_all() -> impl FnMut(u64) -> bool {
        |_| true
    }

    #[test]
    fn metric_names_follow_semantic_convention_shape() {
        for ok in ["system.cpu.time", "container.memory.usage.total", "a", "http/server_duration-x"] {
            assert!(valid_metric_name(ok), "{ok}");
        }
        let long = "a".repeat(256);
        for bad in ["", "1abc", ".cpu", "cpu time", "cpu;drop", "名前", long.as_str()] {
            assert!(!valid_metric_name(bad), "{bad}");
        }
    }

    #[test]
    fn gauges_and_sums_are_kept_and_redacted() {
        let sum = Metric {
            name: "system.network.io".into(),
            unit: "By".into(),
            description: "free text".into(),
            data: Some(Data::Sum(Sum {
                data_points: vec![NumberDataPoint {
                    time_unix_nano: 5,
                    value: Some(number_data_point::Value::AsInt(42)),
                    exemplars: vec![Exemplar { trace_id: vec![1; 16], ..Default::default() }],
                    attributes: vec![kv("direction", "receive"), kv("api_key", "leak")],
                    ..Default::default()
                }],
                aggregation_temporality: 2,
                is_monotonic: true,
            })),
            ..Default::default()
        };
        let mut req = metrics_req(
            vec![kv("host.name", "vps-app"), kv("password", "pw")],
            vec![gauge("system.cpu.utilization", vec![point(1, 0.5, vec![])]), sum],
        );
        let st = sanitize_metrics(&mut req, &mut admit_all());
        assert_eq!((st.accepted, st.rejected, st.over_limit, st.redacted), (2, 0, 0, 2));
        let dump = format!("{req:?}");
        assert!(!dump.contains("leak") && !dump.contains("\"pw\"") && !dump.contains("free text"));
        let m = &req.resource_metrics[0].scope_metrics[0].metrics[1];
        assert_eq!(m.unit, "By");
        let Some(Data::Sum(s)) = &m.data else { panic!("sum expected") };
        assert!(s.data_points[0].exemplars.is_empty());
    }

    #[test]
    fn histogram_points_are_kept_only_with_bounded_consistent_buckets() {
        let valid = HistogramDataPoint {
            time_unix_nano: 5,
            count: 3,
            sum: Some(0.7),
            explicit_bounds: vec![0.1, 0.5],
            bucket_counts: vec![1, 1, 1],
            attributes: vec![kv("http.route", "/orders/{id}"), kv("api_key", "secret")],
            exemplars: vec![Exemplar { trace_id: vec![1; 16], ..Default::default() }],
            ..Default::default()
        };
        let invalid = HistogramDataPoint { count: 2, ..valid.clone() };
        let mut req = metrics_req(
            vec![],
            vec![Metric {
                name: "http.server.request.duration".into(),
                unit: "s".into(),
                data: Some(Data::Histogram(Histogram {
                    data_points: vec![valid, invalid],
                    aggregation_temporality: 2,
                })),
                ..Default::default()
            }],
        );
        let stats = sanitize_metrics(&mut req, &mut admit_all());
        assert_eq!((stats.accepted, stats.rejected, stats.redacted), (1, 1, 1));
        let Some(Data::Histogram(hist)) = &req.resource_metrics[0].scope_metrics[0].metrics[0].data else {
            panic!("histogram expected")
        };
        assert_eq!(hist.data_points.len(), 1);
        assert!(hist.data_points[0].exemplars.is_empty());
        assert!(!format!("{req:?}").contains("secret"));
    }

    #[test]
    fn unsupported_types_invalid_points_and_names_are_counted() {
        let hist = Metric {
            name: "http.server.duration".into(),
            data: Some(Data::Histogram(Histogram {
                data_points: vec![HistogramDataPoint::default(), HistogramDataPoint::default()],
                aggregation_temporality: 2,
            })),
            ..Default::default()
        };
        let summary = Metric {
            name: "rpc.duration".into(),
            data: Some(Data::Summary(Summary { data_points: vec![SummaryDataPoint::default()] })),
            ..Default::default()
        };
        let no_value = NumberDataPoint { time_unix_nano: 1, ..Default::default() };
        let mut req = metrics_req(
            vec![],
            vec![
                hist,
                summary,
                gauge("bad name", vec![point(1, 1.0, vec![])]),
                gauge(
                    "ok.metric",
                    vec![point(0, 1.0, vec![]), point(1, f64::NAN, vec![]), no_value, point(2, 3.0, vec![])],
                ),
                Metric { name: "empty".into(), data: None, ..Default::default() },
            ],
        );
        let st = sanitize_metrics(&mut req, &mut admit_all());
        assert_eq!(st.accepted, 1);
        assert_eq!(st.rejected, 2 + 1 + 1 + 3);
        let metrics = &req.resource_metrics[0].scope_metrics[0].metrics;
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].name, "ok.metric");
    }

    #[test]
    fn empty_groups_are_removed() {
        let mut req = metrics_req(vec![], vec![gauge("x", vec![point(0, 1.0, vec![])])]);
        let st = sanitize_metrics(&mut req, &mut admit_all());
        assert_eq!(st.rejected, 1);
        assert!(req.resource_metrics.is_empty());
    }

    #[test]
    fn series_limit_refusals_are_counted_separately() {
        let points = (0..4).map(|i| point(1, 1.0, vec![kv("cpu", &i.to_string())])).collect();
        let mut req = metrics_req(vec![kv("host.name", "h")], vec![gauge("system.cpu.time", points)]);
        let mut seen = Vec::new();
        let st = sanitize_metrics(&mut req, &mut |k| {
            seen.push(k);
            seen.len() <= 2
        });
        assert_eq!((st.accepted, st.over_limit, st.rejected, st.refused()), (2, 2, 0, 2));
        let unique: std::collections::HashSet<_> = seen.iter().collect();
        assert_eq!(unique.len(), 4, "every attribute set is its own series");
    }

    #[test]
    fn series_key_ignores_attribute_order_but_not_values() {
        let a = series_key("m", &[kv("host.name", "h"), kv("x", "1")], &[kv("cpu", "0"), kv("state", "idle")]);
        let b = series_key("m", &[kv("x", "1"), kv("host.name", "h")], &[kv("state", "idle"), kv("cpu", "0")]);
        assert_eq!(a, b);
        assert_ne!(a, series_key("m", &[kv("host.name", "h"), kv("x", "1")], &[kv("cpu", "1"), kv("state", "idle")]));
        assert_ne!(a, series_key("n", &[kv("host.name", "h"), kv("x", "1")], &[kv("cpu", "0"), kv("state", "idle")]));
        // Resource and point attributes are separated: moving a key across changes the series.
        assert_ne!(series_key("m", &[kv("a", "1")], &[]), series_key("m", &[], &[kv("a", "1")]));
    }

    #[test]
    fn series_key_matches_the_original_hash_for_small_and_large_attribute_sets() {
        fn reference_hash_attrs(h: &mut u64, attrs: &[KeyValue]) {
            let mut sorted: Vec<&KeyValue> = attrs.iter().collect();
            sorted.sort_by(|a, b| a.key.cmp(&b.key));
            for kv in sorted {
                hash_bytes(h, kv.key.as_bytes());
                match kv.value.as_ref().and_then(|v| v.value.as_ref()) {
                    Some(Value::StringValue(s)) => hash_bytes(h, s.as_bytes()),
                    other => hash_bytes(h, format!("{other:?}").as_bytes()),
                }
            }
        }

        fn reference_series_key(name: &str, resource: &[KeyValue], point: &[KeyValue]) -> u64 {
            let mut h = 0xcbf2_9ce4_8422_2325;
            hash_bytes(&mut h, name.as_bytes());
            reference_hash_attrs(&mut h, resource);
            hash_bytes(&mut h, b"|");
            reference_hash_attrs(&mut h, point);
            h
        }

        for resource_len in [0, 1, 4, 8, 9, 64] {
            for point_len in [0, 1, 4, 8, 9, 64] {
                let resource =
                    (0..resource_len).map(|i| kv(&format!("r{}", i % 5), &format!("v{i}"))).collect::<Vec<_>>();
                let point = (0..point_len).map(|i| kv(&format!("p{}", i % 7), &format!("v{i}"))).collect::<Vec<_>>();
                assert_eq!(
                    series_key("system.cpu.utilization", &resource, &point),
                    reference_series_key("system.cpu.utilization", &resource, &point),
                    "resource attrs: {resource_len}, point attrs: {point_len}"
                );
            }
        }
    }

    #[test]
    fn point_budget_rejects_the_overflow() {
        let points = (0..MAX_POINTS_PER_REQUEST + 3).map(|_| point(1, 1.0, vec![])).collect();
        let mut req = metrics_req(vec![], vec![gauge("m", points)]);
        let st = sanitize_metrics(&mut req, &mut admit_all());
        assert_eq!(st.accepted as usize, MAX_POINTS_PER_REQUEST);
        assert_eq!(st.rejected, 3);
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
                let sensitive = !matches!(karbird_domain::redaction::classify_key(&kv.key), AttributeDecision::Keep);
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

            #[test]
            fn metric_point_accounting_is_complete(
                pts in prop::collection::vec((0u64..3, prop_oneof![Just(f64::NAN), Just(f64::INFINITY), -1e9f64..1e9], 0u8..4), 0..60),
                limit in 0usize..5,
            ) {
                let n = pts.len() as u64;
                let points = pts.into_iter().map(|(t, v, a)| point(t, v, vec![kv("k", &a.to_string())])).collect();
                let mut req = metrics_req(vec![], vec![gauge("m", points)]);
                let mut admitted = std::collections::HashSet::new();
                let st = sanitize_metrics(&mut req, &mut |k| admitted.contains(&k) || (admitted.len() < limit && admitted.insert(k)));
                prop_assert_eq!(st.accepted + st.rejected + st.over_limit, n);
                let kept: u64 = req.resource_metrics.iter().flat_map(|r| &r.scope_metrics).flat_map(|s| &s.metrics)
                    .map(|m| match &m.data { Some(Data::Gauge(g)) => g.data_points.len() as u64, _ => 0 }).sum();
                prop_assert_eq!(kept, st.accepted);
            }
        }
    }
}
