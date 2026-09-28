//! Normalisation of broker envelopes into ClickHouse rows.

use controlhub_domain::{EntityId, EnvironmentId, ProjectId, Scope, ServiceId, TenantId};
use controlhub_protocol::ENVELOPE_SCHEMA_VERSION;
use controlhub_protocol::otlp::collector::logs::v1::ExportLogsServiceRequest;
use controlhub_protocol::otlp::collector::trace::v1::ExportTraceServiceRequest;
use controlhub_protocol::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
use controlhub_protocol::v1::{IngestEnvelope, ingest_envelope::Payload};
use controlhub_storage::ch::{EventRow, LogRow, SpanRow};
use prost::Message;

#[derive(Debug, Default)]
pub struct Converted {
    pub spans: Vec<SpanRow>,
    pub logs: Vec<LogRow>,
    pub events: Vec<EventRow>,
}

impl Converted {
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty() && self.logs.is_empty() && self.events.is_empty()
    }

    pub fn extend(&mut self, other: Converted) {
        self.spans.extend(other.spans);
        self.logs.extend(other.logs);
        self.events.extend(other.events);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("malformed envelope: {0}")]
    Decode(String),
    #[error("unsupported envelope schema_version {0}")]
    UnsupportedVersion(u32),
    #[error("envelope has no valid scope: {0}")]
    Scope(String),
    #[error("envelope has no payload")]
    EmptyPayload,
}

impl ConvertError {
    pub fn kind(&self) -> &'static str {
        match self {
            ConvertError::Decode(_) => "decode",
            ConvertError::UnsupportedVersion(_) => "unsupported_version",
            ConvertError::Scope(_) => "invalid_scope",
            ConvertError::EmptyPayload => "empty_payload",
        }
    }
}

pub fn decode_envelope(bytes: &[u8]) -> Result<IngestEnvelope, ConvertError> {
    IngestEnvelope::decode(bytes).map_err(|e| ConvertError::Decode(e.to_string()))
}

pub fn any_to_string(v: &AnyValue) -> String {
    match &v.value {
        Some(Value::StringValue(s)) => s.clone(),
        Some(Value::BoolValue(b)) => b.to_string(),
        Some(Value::IntValue(i)) => i.to_string(),
        Some(Value::DoubleValue(d)) => d.to_string(),
        Some(Value::BytesValue(b)) => hex::encode(b),
        Some(Value::ArrayValue(a)) => {
            let items: Vec<String> = a.values.iter().map(any_to_string).collect();
            serde_json::to_string(&items).unwrap_or_default()
        }
        Some(Value::KvlistValue(kv)) => {
            let map: serde_json::Map<String, serde_json::Value> = kv
                .values
                .iter()
                .map(|x| {
                    (x.key.clone(), serde_json::Value::String(x.value.as_ref().map(any_to_string).unwrap_or_default()))
                })
                .collect();
            serde_json::Value::Object(map).to_string()
        }
        #[allow(unreachable_patterns)]
        _ => String::new(),
    }
}

fn flatten(attrs: &[KeyValue]) -> Vec<(String, String)> {
    attrs.iter().map(|kv| (kv.key.clone(), kv.value.as_ref().map(any_to_string).unwrap_or_default())).collect()
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

struct ServiceIdentity {
    id: String,
    name: String,
    namespace: String,
    instance: String,
    version: String,
}

fn service_identity(scope: &Scope, resource: &[(String, String)]) -> ServiceIdentity {
    let name = attr(resource, "service.name").unwrap_or("unknown_service").to_owned();
    let namespace = attr(resource, "service.namespace").unwrap_or("").to_owned();
    ServiceIdentity {
        id: ServiceId::derive(scope, &namespace, &name).to_string(),
        instance: attr(resource, "service.instance.id").unwrap_or("").to_owned(),
        version: attr(resource, "service.version").unwrap_or("").to_owned(),
        name,
        namespace,
    }
}

/// Dependency target for client/producer spans, derived from semantic
/// conventions. Returned as (entity_id, display name, kind).
fn span_target(scope: &Scope, kind: i32, attrs: &[(String, String)]) -> Option<(String, String, String)> {
    if kind != 3 && kind != 4 {
        return None;
    }
    if let Some(system) = attr(attrs, "db.system.name").or_else(|| attr(attrs, "db.system")) {
        let name = attr(attrs, "db.namespace")
            .or_else(|| attr(attrs, "db.name"))
            .or_else(|| attr(attrs, "server.address"))
            .or_else(|| attr(attrs, "net.peer.name"))
            .unwrap_or(system);
        let id = EntityId::derive_resource(scope, "database", system, name);
        return Some((id.to_string(), format!("{system}: {name}"), "database".into()));
    }
    if let Some(system) = attr(attrs, "messaging.system") {
        let name = attr(attrs, "messaging.destination.name").unwrap_or(system);
        let id = EntityId::derive_resource(scope, "queue", system, name);
        return Some((id.to_string(), format!("{system}: {name}"), "queue".into()));
    }
    None
}

fn scope_of(env: &IngestEnvelope) -> Result<Scope, ConvertError> {
    let s = env.scope.as_ref().ok_or_else(|| ConvertError::Scope("missing".into()))?;
    let err = |e: controlhub_domain::IdError| ConvertError::Scope(e.to_string());
    Ok(Scope {
        tenant_id: TenantId::parse(s.tenant_id.clone()).map_err(err)?,
        project_id: ProjectId::parse(s.project_id.clone()).map_err(err)?,
        environment_id: EnvironmentId::parse(s.environment_id.clone()).map_err(err)?,
    })
}

fn clamp_ns(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

pub fn convert(env: &IngestEnvelope, ingested_at: i64) -> Result<Converted, ConvertError> {
    if env.schema_version == 0 || env.schema_version > ENVELOPE_SCHEMA_VERSION {
        return Err(ConvertError::UnsupportedVersion(env.schema_version));
    }
    let scope = scope_of(env)?;
    let (t, p, e) = (scope.tenant_id.to_string(), scope.project_id.to_string(), scope.environment_id.to_string());
    let mut out = Converted::default();
    match env.payload.as_ref().ok_or(ConvertError::EmptyPayload)? {
        Payload::OtlpTraces(bytes) => {
            let req =
                ExportTraceServiceRequest::decode(bytes.as_slice()).map_err(|e| ConvertError::Decode(e.to_string()))?;
            for rs in req.resource_spans {
                let resource = rs.resource.map(|r| flatten(&r.attributes)).unwrap_or_default();
                let svc = service_identity(&scope, &resource);
                for ss in rs.scope_spans {
                    for span in ss.spans {
                        let attributes = flatten(&span.attributes);
                        let target = span_target(&scope, span.kind, &attributes);
                        let (target_entity_id, target_name, target_kind) = target.unwrap_or_default();
                        let start = clamp_ns(span.start_time_unix_nano);
                        let end = clamp_ns(span.end_time_unix_nano);
                        let status = span.status.unwrap_or_default();
                        out.spans.push(SpanRow {
                            tenant_id: t.clone(),
                            project_id: p.clone(),
                            environment_id: e.clone(),
                            service_id: svc.id.clone(),
                            service_name: svc.name.clone(),
                            service_namespace: svc.namespace.clone(),
                            service_instance_id: svc.instance.clone(),
                            service_version: svc.version.clone(),
                            trace_id: hex::encode(&span.trace_id),
                            span_id: hex::encode(&span.span_id),
                            parent_span_id: hex::encode(&span.parent_span_id),
                            name: span.name,
                            kind: u8::try_from(span.kind).unwrap_or(0),
                            start_time: start,
                            end_time: end,
                            duration_ns: end.saturating_sub(start),
                            status_code: u8::try_from(status.code).unwrap_or(0),
                            status_message: status.message,
                            target_entity_id,
                            target_name,
                            target_kind,
                            attributes,
                            resource_attributes: resource.clone(),
                            batch_id: env.batch_id.clone(),
                            ingested_at,
                        });
                    }
                }
            }
        }
        Payload::OtlpLogs(bytes) => {
            let req =
                ExportLogsServiceRequest::decode(bytes.as_slice()).map_err(|e| ConvertError::Decode(e.to_string()))?;
            for (ri, rl) in req.resource_logs.into_iter().enumerate() {
                let resource = rl.resource.map(|r| flatten(&r.attributes)).unwrap_or_default();
                let svc = service_identity(&scope, &resource);
                for (si, sl) in rl.scope_logs.into_iter().enumerate() {
                    for (li, lr) in sl.log_records.into_iter().enumerate() {
                        let time = if lr.time_unix_nano > 0 { lr.time_unix_nano } else { lr.observed_time_unix_nano };
                        out.logs.push(LogRow {
                            tenant_id: t.clone(),
                            project_id: p.clone(),
                            environment_id: e.clone(),
                            service_id: svc.id.clone(),
                            service_name: svc.name.clone(),
                            service_instance_id: svc.instance.clone(),
                            log_id: format!("{}-{ri}-{si}-{li}", env.batch_id),
                            time: clamp_ns(time),
                            observed_time: clamp_ns(lr.observed_time_unix_nano),
                            severity_number: u8::try_from(lr.severity_number).unwrap_or(0),
                            severity_text: lr.severity_text,
                            body: lr.body.as_ref().map(any_to_string).unwrap_or_default(),
                            trace_id: hex::encode(&lr.trace_id),
                            span_id: hex::encode(&lr.span_id),
                            attributes: flatten(&lr.attributes),
                            resource_attributes: resource.clone(),
                            batch_id: env.batch_id.clone(),
                            ingested_at,
                        });
                    }
                }
            }
        }
        Payload::CustomEvents(batch) => {
            for ev in &batch.events {
                let service_id = ServiceId::derive(&scope, &ev.service_namespace, &ev.service_name).to_string();
                let mut attributes: Vec<(String, String)> =
                    ev.attributes.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                attributes.sort();
                out.events.push(EventRow {
                    tenant_id: t.clone(),
                    project_id: p.clone(),
                    environment_id: e.clone(),
                    event_id: ev.event_id.clone(),
                    event_type: ev.event_type.clone(),
                    service_id,
                    service_name: ev.service_name.clone(),
                    service_instance_id: ev.service_instance_id.clone(),
                    entity_id: ev.entity_id.clone(),
                    event_time: ev.event_time_unix_nano,
                    observed_time: if ev.observed_time_unix_nano > 0 {
                        ev.observed_time_unix_nano
                    } else {
                        env.received_time_unix_nano
                    },
                    trace_id: ev.trace_id.clone(),
                    span_id: ev.span_id.clone(),
                    source_id: ev.source_id.clone(),
                    source_epoch: ev.source_epoch.clone(),
                    source_sequence: ev.source_sequence,
                    attributes,
                    payload_json: ev.payload_json.clone(),
                    batch_id: env.batch_id.clone(),
                    ingested_at,
                });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use controlhub_protocol::otlp::resource::v1::Resource;
    use controlhub_protocol::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use controlhub_protocol::v1::IngestScope;

    fn kv(k: &str, v: &str) -> KeyValue {
        KeyValue {
            key: k.into(),
            value: Some(AnyValue { value: Some(Value::StringValue(v.into())) }),
            ..Default::default()
        }
    }

    fn envelope(tenant: &str, resource_attrs: Vec<KeyValue>) -> IngestEnvelope {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource { attributes: resource_attrs, ..Default::default() }),
                scope_spans: vec![ScopeSpans {
                    spans: vec![Span {
                        trace_id: vec![1; 16],
                        span_id: vec![2; 8],
                        kind: 3,
                        name: "SELECT orders".into(),
                        start_time_unix_nano: 100,
                        end_time_unix_nano: 250,
                        attributes: vec![kv("db.system", "postgresql"), kv("db.name", "orders")],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        IngestEnvelope {
            schema_version: 1,
            batch_id: "b1".into(),
            scope: Some(IngestScope {
                tenant_id: tenant.into(),
                project_id: "p1".into(),
                environment_id: "dev".into(),
                token_id: "tok".into(),
            }),
            received_time_unix_nano: 1,
            source_id: "ingest-1".into(),
            payload: Some(Payload::OtlpTraces(req.encode_to_vec())),
        }
    }

    #[test]
    fn converts_spans_with_db_target() {
        let env = envelope("t1", vec![kv("service.name", "orders")]);
        let out = convert(&env, 5).unwrap();
        assert_eq!(out.spans.len(), 1);
        let s = &out.spans[0];
        assert_eq!(s.trace_id, "01".repeat(16));
        assert_eq!(s.duration_ns, 150);
        assert_eq!(s.target_kind, "database");
        assert_eq!(s.target_name, "postgresql: orders");
        assert!(s.target_entity_id.starts_with("res_"));
    }

    #[test]
    fn scope_comes_from_envelope_not_payload() {
        // A payload claiming another tenant in resource attributes cannot change the scope.
        let env =
            envelope("t1", vec![kv("service.name", "orders"), kv("tenant_id", "t2"), kv("controlhub.tenant_id", "t2")]);
        let out = convert(&env, 5).unwrap();
        assert_eq!(out.spans[0].tenant_id, "t1");
        let other = convert(&envelope("t2", vec![kv("service.name", "orders")]), 5).unwrap();
        assert_ne!(out.spans[0].service_id, other.spans[0].service_id);
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let mut env = envelope("t1", vec![]);
        env.schema_version = 99;
        assert!(matches!(convert(&env, 1), Err(ConvertError::UnsupportedVersion(99))));
    }
}
