//! `POST /api/v1/events/batch` payload parsing and validation.

use std::collections::BTreeMap;

use controlhub_domain::model::{event_types, limits};
use controlhub_domain::redaction::{AttributeDecision, REDACTED, classify_key, sanitize_value, truncate};
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
            other => other.to_string(),
        };
        match classify_key(&key) {
            AttributeDecision::Drop => {}
            AttributeDecision::Redact => {
                attributes.insert(key, REDACTED.to_owned());
            }
            AttributeDecision::Keep => {
                attributes.insert(key, sanitize_value(&value));
            }
        }
    }
    let payload_json = match e.payload {
        None => String::new(),
        Some(p) => {
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
}
