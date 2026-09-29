//! `POST /api/v1/events/batch` payload parsing and validation.

use std::collections::BTreeMap;

use controlhub_domain::model::{event_types, limits};
use controlhub_domain::redaction::{
    AttributeDecision, REDACTED, classify_key, mask_bearer, sanitize_attribute, truncate,
};
use controlhub_domain::time::{nanos_string, opt_nanos_string};
use controlhub_protocol::v1::{CustomEvent, CustomEventBatch};
use serde::{Deserialize, Serialize};

const MAX_PAYLOAD_JSON: usize = 8 * 1024;

#[derive(Debug, Deserialize)]
pub struct EventBatchIn {
    pub events: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ServiceIn {
    name: String,
    #[serde(default)]
    namespace: String,
    #[serde(default)]
    instance_id: String,
}

#[derive(Debug, Deserialize, Default)]
struct SourceIn {
    #[serde(default)]
    id: String,
    #[serde(default)]
    epoch: String,
    #[serde(default)]
    sequence: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct EventIn {
    event_id: String,
    event_type: String,
    service: ServiceIn,
    #[serde(default)]
    entity_id: Option<String>,
    #[serde(with = "nanos_string")]
    event_time: i64,
    #[serde(default, with = "opt_nanos_string")]
    observed_time: Option<i64>,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    span_id: Option<String>,
    #[serde(default)]
    source: Option<SourceIn>,
    #[serde(default)]
    attributes: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Rejected {
    pub index: usize,
    pub reason: String,
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_token(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_graphic())
}

/// Apply attribute redaction rules to keys nested inside JSON values (structured attribute
/// values and the event payload). Depth is bounded by serde_json's parser recursion limit.
fn redact_json(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.retain(|k, _| classify_key(k) != AttributeDecision::Drop);
            for (k, val) in map.iter_mut() {
                if classify_key(k) == AttributeDecision::Redact {
                    *val = serde_json::Value::String(REDACTED.to_owned());
                } else {
                    redact_json(val);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_json),
        serde_json::Value::String(s) => *s = mask_bearer(s),
        _ => {}
    }
}

fn convert(e: EventIn) -> Result<CustomEvent, String> {
    if !valid_token(&e.event_id, 128) {
        return Err("event_id must be 1..128 printable ASCII characters".into());
    }
    if !event_types::is_valid(&e.event_type) {
        return Err("event_type must be a dotted lowercase identifier".into());
    }
    if !valid_token(&e.service.name, 256) {
        return Err("service.name is required".into());
    }
    if e.event_time <= 0 {
        return Err("event_time must be a positive nanosecond timestamp string".into());
    }
    let trace_id = e.trace_id.unwrap_or_default();
    if !trace_id.is_empty() && !is_hex(&trace_id, 32) {
        return Err("trace_id must be 32 lowercase hex characters".into());
    }
    let span_id = e.span_id.unwrap_or_default();
    if !span_id.is_empty() && !is_hex(&span_id, 16) {
        return Err("span_id must be 16 lowercase hex characters".into());
    }
    if e.attributes.len() > limits::MAX_ATTRIBUTES_PER_ITEM {
        return Err(format!("at most {} attributes", limits::MAX_ATTRIBUTES_PER_ITEM));
    }
    let mut attributes = std::collections::HashMap::new();
    for (k, v) in e.attributes {
        let key = truncate(&k, limits::MAX_ATTRIBUTE_KEY_LEN).to_owned();
        let value = match v {
            serde_json::Value::String(s) => s,
            mut other => {
                redact_json(&mut other);
                other.to_string()
            }
        };
        // Classify the full key: a sensitive fragment past the length limit must still match.
        match classify_key(&k) {
            AttributeDecision::Drop => {}
            AttributeDecision::Redact => {
                attributes.insert(key, REDACTED.to_owned());
            }
            AttributeDecision::Keep => {
                attributes.insert(key, sanitize_attribute(&k, &value).0);
            }
        }
    }
    let payload_json = match e.payload {
        None => String::new(),
        Some(mut p) => {
            redact_json(&mut p);
            let s = p.to_string();
            if s.len() > MAX_PAYLOAD_JSON {
                return Err(format!("payload exceeds {MAX_PAYLOAD_JSON} bytes"));
            }
            s
        }
    };
    let source = e.source.unwrap_or_default();
    Ok(CustomEvent {
        event_id: e.event_id,
        event_type: e.event_type,
        service_name: e.service.name,
        service_namespace: truncate(&e.service.namespace, 256).to_owned(),
        service_instance_id: truncate(&e.service.instance_id, 256).to_owned(),
        entity_id: e.entity_id.map(|s| truncate(&s, 128).to_owned()).unwrap_or_default(),
        event_time_unix_nano: e.event_time,
        observed_time_unix_nano: e.observed_time.unwrap_or(0),
        trace_id,
        span_id,
        source_id: truncate(&source.id, 128).to_owned(),
        source_epoch: truncate(&source.epoch, 128).to_owned(),
        source_sequence: source.sequence,
        attributes,
        payload_json,
    })
}

/// Parse a batch. Individually invalid events are rejected with a reason;
/// a malformed envelope fails the whole request.
pub fn parse_batch(body: &[u8]) -> Result<(CustomEventBatch, Vec<Rejected>), String> {
    let batch: EventBatchIn = serde_json::from_slice(body).map_err(|e| format!("malformed JSON: {e}"))?;
    if batch.events.len() > limits::MAX_EVENTS_PER_BATCH {
        return Err(format!("at most {} events per batch", limits::MAX_EVENTS_PER_BATCH));
    }
    let mut out = CustomEventBatch::default();
    let mut rejected = Vec::new();
    for (index, raw) in batch.events.into_iter().enumerate() {
        match serde_json::from_value::<EventIn>(raw) {
            Ok(e) => match convert(e) {
                Ok(ev) => out.events.push(ev),
                Err(reason) => rejected.push(Rejected { index, reason }),
            },
            Err(err) => rejected.push(Rejected { index, reason: format!("invalid event: {err}") }),
        }
    }
    Ok((out, rejected))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn parses_and_redacts() {
        let body = br#"{"events":[
          {"event_id":"e1","event_type":"request.started","service":{"name":"gateway"},
           "event_time":"1790000000123456789","attributes":{"http.route":"/orders","api_key":"sk-123"},
           "tenant_id":"someone-else"},
          {"event_id":"e2","event_type":"Bad Type","service":{"name":"gateway"},"event_time":"1"}
        ]}"#;
        let (batch, rejected) = parse_batch(body).unwrap();
        assert_eq!(batch.events.len(), 1);
        assert_eq!(rejected.len(), 1);
        let ev = &batch.events[0];
        assert_eq!(ev.event_time_unix_nano, 1_790_000_000_123_456_789);
        assert_eq!(ev.attributes.get("api_key").unwrap(), REDACTED);
        assert_eq!(ev.attributes.get("http.route").unwrap(), "/orders");
    }

    #[test]
    fn rejects_oversized_batch() {
        let events: Vec<_> = (0..1001).map(|_| serde_json::json!({})).collect();
        let body = serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap();
        assert!(parse_batch(&body).is_err());
    }

    use serde_json::{Value, json};

    fn base() -> Value {
        json!({
            "event_id": "e1",
            "event_type": "deployment.started",
            "service": {"name": "orders"},
            "event_time": "1790000000000000000"
        })
    }

    fn with(patch: Value) -> Value {
        let mut ev = base();
        if let (Some(obj), Some(p)) = (ev.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                if v.is_null() {
                    obj.remove(k);
                } else {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        ev
    }

    fn one(ev: Value) -> Result<CustomEvent, String> {
        let body = serde_json::to_vec(&json!({ "events": [ev] })).unwrap();
        let (mut batch, rejected) = parse_batch(&body)?;
        match rejected.into_iter().next() {
            Some(r) => Err(r.reason),
            None => Ok(batch.events.remove(0)),
        }
    }

    #[test]
    fn minimal_event_gets_defaults() {
        let ev = one(base()).unwrap();
        assert_eq!(ev.event_id, "e1");
        assert_eq!(ev.service_name, "orders");
        assert_eq!(ev.observed_time_unix_nano, 0);
        assert!(ev.trace_id.is_empty() && ev.span_id.is_empty() && ev.entity_id.is_empty());
        assert_eq!(ev.source_sequence, None);
        assert!(ev.payload_json.is_empty());
        assert!(ev.attributes.is_empty());
    }

    #[test]
    fn full_event_maps_every_field() {
        let ev = one(with(json!({
            "service": {"name": "orders", "namespace": "shop", "instance_id": "pod-1"},
            "entity_id": "svc_1",
            "observed_time": "1790000000000000005",
            "trace_id": "0123456789abcdef0123456789abcdef",
            "span_id": "0123456789abcdef",
            "source": {"id": "sdk-1", "epoch": "boot-7", "sequence": 0},
            "attributes": {"n": 5, "b": true, "s": "x", "arr": [1, "two"]},
            "payload": {"version": "1.2.3"}
        })))
        .unwrap();
        assert_eq!(ev.service_namespace, "shop");
        assert_eq!(ev.service_instance_id, "pod-1");
        assert_eq!(ev.entity_id, "svc_1");
        assert_eq!(ev.observed_time_unix_nano, 1_790_000_000_000_000_005);
        assert_eq!(ev.trace_id, "0123456789abcdef0123456789abcdef");
        assert_eq!(ev.span_id, "0123456789abcdef");
        assert_eq!((ev.source_id.as_str(), ev.source_epoch.as_str()), ("sdk-1", "boot-7"));
        assert_eq!(ev.source_sequence, Some(0));
        assert_eq!(ev.attributes["n"], "5");
        assert_eq!(ev.attributes["b"], "true");
        assert_eq!(ev.attributes["s"], "x");
        assert_eq!(ev.attributes["arr"], r#"[1,"two"]"#);
        assert_eq!(ev.payload_json, r#"{"version":"1.2.3"}"#);
    }

    #[test]
    fn each_validation_rule_rejects_with_a_reason() {
        let cases = [
            (json!({"event_id": ""}), "event_id"),
            (json!({"event_id": "a b"}), "event_id"),
            (json!({"event_id": "x".repeat(129)}), "event_id"),
            (json!({"event_id": "é"}), "event_id"),
            (json!({"event_type": "Deploy"}), "event_type"),
            (json!({"event_type": ""}), "event_type"),
            (json!({"service": {"name": ""}}), "service.name"),
            (json!({"service": {"name": "has space"}}), "service.name"),
            (json!({"service": {"name": "x".repeat(257)}}), "service.name"),
            (json!({"event_time": "0"}), "event_time"),
            (json!({"event_time": "-1"}), "event_time"),
            (json!({"trace_id": "0123456789ABCDEF0123456789ABCDEF"}), "trace_id"),
            (json!({"trace_id": "0123"}), "trace_id"),
            (json!({"span_id": "0123456789abcdeZ"}), "span_id"),
            (json!({"payload": {"blob": "x".repeat(MAX_PAYLOAD_JSON)}}), "payload"),
        ];
        for (patch, field) in cases {
            let err = one(with(patch.clone())).unwrap_err();
            assert!(err.contains(field), "{patch}: {err}");
        }
        assert!(one(with(json!({"event_id": "x".repeat(128)}))).is_ok());
    }

    #[test]
    fn structurally_invalid_events_are_rejected_individually() {
        for patch in [
            json!({"event_id": null}),
            json!({"service": null}),
            json!({"event_time": null}),
            json!({"event_time": "soon"}),
            json!({"event_time": 1.5}),
            json!({"attributes": []}),
            json!({"source": {"sequence": -1}}),
        ] {
            let err = one(with(patch.clone())).unwrap_err();
            assert!(err.starts_with("invalid event:"), "{patch}: {err}");
        }
        let body = br#"{"events":[1, "x", null, []]}"#;
        let (batch, rejected) = parse_batch(body).unwrap();
        assert!(batch.events.is_empty());
        assert_eq!(rejected.iter().map(|r| r.index).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn malformed_envelope_fails_the_whole_request() {
        for body in [&b""[..], b"{", b"[]", b"{}", br#"{"events":{}}"#, b"\xff\xfe", br#"{"events":null}"#] {
            let err = parse_batch(body).unwrap_err();
            assert!(err.starts_with("malformed JSON"), "{err}");
        }
        let (batch, rejected) = parse_batch(br#"{"events":[]}"#).unwrap();
        assert!(batch.events.is_empty() && rejected.is_empty());
    }

    #[test]
    fn rejected_indexes_point_at_the_right_events() {
        let bad = with(json!({"event_type": "BAD"}));
        let body = serde_json::to_vec(&json!({ "events": [base(), bad.clone(), base(), bad] })).unwrap();
        let (batch, rejected) = parse_batch(&body).unwrap();
        assert_eq!(batch.events.len(), 2);
        assert_eq!(rejected.iter().map(|r| r.index).collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn exactly_max_events_is_allowed() {
        let events: Vec<_> = (0..limits::MAX_EVENTS_PER_BATCH).map(|_| base()).collect();
        let body = serde_json::to_vec(&json!({ "events": events })).unwrap();
        let (batch, _) = parse_batch(&body).unwrap();
        assert_eq!(batch.events.len(), limits::MAX_EVENTS_PER_BATCH);
    }

    #[test]
    fn attribute_limits() {
        let at_limit: serde_json::Map<String, Value> =
            (0..limits::MAX_ATTRIBUTES_PER_ITEM).map(|i| (format!("k{i}"), json!("v"))).collect();
        assert!(one(with(json!({ "attributes": at_limit }))).is_ok());
        let over: serde_json::Map<String, Value> =
            (0..=limits::MAX_ATTRIBUTES_PER_ITEM).map(|i| (format!("k{i}"), json!("v"))).collect();
        assert!(one(with(json!({ "attributes": over }))).unwrap_err().contains("attributes"));

        let long_key = "k".repeat(limits::MAX_ATTRIBUTE_KEY_LEN + 50);
        let huge = "v".repeat(limits::MAX_ATTRIBUTE_VALUE_LEN * 2);
        let ev = one(with(json!({ "attributes": { long_key: huge } }))).unwrap();
        let (k, v) = ev.attributes.iter().next().unwrap();
        assert_eq!(k.len(), limits::MAX_ATTRIBUTE_KEY_LEN);
        assert_eq!(v.len(), limits::MAX_ATTRIBUTE_VALUE_LEN);
    }

    #[test]
    fn sensitive_attributes_are_redacted_or_dropped() {
        let ev = one(with(json!({ "attributes": {
            "Authorization": "Bearer abc",
            "COOKIE": "sid=1",
            "user.password": "hunter2",
            "db.statement": "SELECT * FROM users",
            "http.request.body": "{}",
            "message": "retry with Bearer xyz"
        }})))
        .unwrap();
        assert_eq!(ev.attributes["Authorization"], REDACTED);
        assert_eq!(ev.attributes["COOKIE"], REDACTED);
        assert_eq!(ev.attributes["user.password"], REDACTED);
        assert!(!ev.attributes.contains_key("db.statement"));
        assert!(!ev.attributes.contains_key("http.request.body"));
        assert_eq!(ev.attributes["message"], "retry with Bearer [REDACTED]");
    }

    #[test]
    fn sensitive_fragment_beyond_key_limit_is_still_redacted() {
        // Regression: keys were truncated before classification.
        let key = format!("{}_api_key", "a".repeat(limits::MAX_ATTRIBUTE_KEY_LEN));
        let ev = one(with(json!({ "attributes": { key: "sk-live-123" } }))).unwrap();
        let v = ev.attributes.values().next().unwrap();
        assert_eq!(v, REDACTED);
    }

    #[test]
    fn nested_attribute_objects_are_redacted() {
        // Regression: structured values were stringified verbatim, leaking nested secrets.
        let ev = one(with(json!({ "attributes": {
            "db": {"host": "pg", "password": "hunter2", "statement": {"db.statement": "SELECT 1"}},
            "headers": [{"authorization": "Bearer abc"}, {"accept": "json"}]
        }})))
        .unwrap();
        let db = &ev.attributes["db"];
        assert!(!db.contains("hunter2"), "{db}");
        assert!(!db.contains("SELECT 1"), "{db}");
        assert!(db.contains("\"host\":\"pg\""), "{db}");
        let headers = &ev.attributes["headers"];
        assert!(!headers.contains("abc"), "{headers}");
        assert!(headers.contains("accept"), "{headers}");
    }

    #[test]
    fn payload_is_redacted() {
        // Regression: the payload was stored verbatim.
        let ev = one(with(json!({ "payload": {
            "api_key": "sk-123",
            "http.request.body": "raw",
            "nested": [{"Cookie": "sid"}, "Bearer tok123"],
            "version": "1.0"
        }})))
        .unwrap();
        let p: Value = serde_json::from_str(&ev.payload_json).unwrap();
        assert_eq!(p["api_key"], REDACTED);
        assert!(p.get("http.request.body").is_none());
        assert_eq!(p["nested"][0]["Cookie"], REDACTED);
        assert_eq!(p["nested"][1], "Bearer [REDACTED]");
        assert_eq!(p["version"], "1.0");
        for leaked in ["sk-123", "raw", "sid", "tok123"] {
            assert!(!ev.payload_json.contains(leaked), "{leaked}");
        }
    }

    #[test]
    fn scalar_payloads_are_kept() {
        assert_eq!(one(with(json!({ "payload": 5 }))).unwrap().payload_json, "5");
        assert_eq!(one(with(json!({ "payload": "Basic abc" }))).unwrap().payload_json, r#""Basic [REDACTED]""#);
    }

    #[test]
    fn free_text_fields_are_truncated() {
        let ev = one(with(json!({
            "service": {"name": "orders", "namespace": "n".repeat(300), "instance_id": "i".repeat(300)},
            "entity_id": "e".repeat(300),
            "source": {"id": "s".repeat(300), "epoch": "p".repeat(300)}
        })))
        .unwrap();
        assert_eq!(ev.service_namespace.len(), 256);
        assert_eq!(ev.service_instance_id.len(), 256);
        assert_eq!(ev.entity_id.len(), 128);
        assert_eq!(ev.source_id.len(), 128);
        assert_eq!(ev.source_epoch.len(), 128);
    }

    #[test]
    fn scope_fields_in_the_payload_are_ignored() {
        let ev = one(with(json!({"tenant_id": "evil", "project_id": "evil", "environment_id": "evil"}))).unwrap();
        let encoded = format!("{ev:?}");
        assert!(!encoded.contains("evil"));
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn arbitrary_bytes_never_panic(body in prop::collection::vec(any::<u8>(), 0..512)) {
                let _ = parse_batch(&body);
            }

            #[test]
            fn secret_keys_never_keep_their_value(
                prefix in "[a-zA-Z._-]{0,150}",
                fragment in prop::sample::select(vec!["password", "Authorization", "COOKIE", "token", "api-key", "secret"]),
                nested in any::<bool>(),
            ) {
                let key = format!("{prefix}{fragment}");
                let value = if nested { json!({"inner": "SECRETVALUE"}) } else { json!("SECRETVALUE") };
                let mut attrs = serde_json::Map::new();
                attrs.insert(key, value.clone());
                let ev = one(with(json!({ "attributes": attrs, "payload": { "x": { fragment: value } } }))).unwrap();
                let dump = format!("{ev:?}");
                prop_assert!(!dump.contains("SECRETVALUE"));
            }
        }
    }
}
