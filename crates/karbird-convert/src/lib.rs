//! Normalisation of broker envelopes into ClickHouse rows.

use std::collections::HashMap;

use karbird_domain::{EntityId, EnvironmentId, ProjectId, Scope, ServiceId, TenantId};
use karbird_protocol::ENVELOPE_SCHEMA_VERSION;
use karbird_protocol::otlp::collector::logs::v1::ExportLogsServiceRequest;
use karbird_protocol::otlp::collector::metrics::v1::ExportMetricsServiceRequest;
use karbird_protocol::otlp::collector::trace::v1::ExportTraceServiceRequest;
use karbird_protocol::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
use karbird_protocol::otlp::metrics::v1::{NumberDataPoint, metric::Data, number_data_point};
use karbird_protocol::v1::{IngestEnvelope, ingest_envelope::Payload};
use karbird_storage::ch::{
    EventRow, LogRow, METRIC_KIND_CUMULATIVE_SUM, METRIC_KIND_DELTA_SUM, METRIC_KIND_GAUGE, MetricRow, SpanRow,
};
use prost::Message;
use sha2::{Digest, Sha256};

#[derive(Debug, Default)]
pub struct Converted {
    pub spans: Vec<SpanRow>,
    pub logs: Vec<LogRow>,
    pub events: Vec<EventRow>,
    pub metrics: Vec<MetricRow>,
    /// Metric points that are not gauges/sums or carry no finite value; ingest already rejects
    /// them, so a non-zero count means an envelope bypassed it.
    pub dropped_points: u64,
}

impl Converted {
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty() && self.logs.is_empty() && self.events.is_empty() && self.metrics.is_empty()
    }

    pub fn extend(&mut self, other: Converted) {
        self.spans.extend(other.spans);
        self.logs.extend(other.logs);
        self.events.extend(other.events);
        self.metrics.extend(other.metrics);
        self.dropped_points += other.dropped_points;
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

/// Deterministic log ID from the record's content, so a client retry after a lost ACK maps to
/// the same row (ReplacingMergeTree collapses it). Identical records from the same service
/// instance at the same nanosecond are treated as one.
fn log_fingerprint(
    svc: &ServiceIdentity,
    time: u64,
    lr: &karbird_protocol::otlp::logs::v1::LogRecord,
    body: &str,
    attributes: &[(String, String)],
) -> String {
    let mut h = Sha256::new();
    let mut field = |b: &[u8]| {
        h.update((b.len() as u64).to_le_bytes());
        h.update(b);
    };
    field(svc.id.as_bytes());
    field(svc.instance.as_bytes());
    field(&time.to_le_bytes());
    field(&lr.observed_time_unix_nano.to_le_bytes());
    field(&lr.severity_number.to_le_bytes());
    field(lr.severity_text.as_bytes());
    field(body.as_bytes());
    field(&lr.trace_id);
    field(&lr.span_id);
    let mut attrs: Vec<&(String, String)> = attributes.iter().collect();
    attrs.sort();
    for (k, v) in attrs {
        field(k.as_bytes());
        field(v.as_bytes());
    }
    hex::encode(&h.finalize()[..16])
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
    if kind == 3
        && (attr(attrs, "http.request.method").is_some() || attr(attrs, "http.method").is_some())
        && let Some(host) = http_peer_host(attrs)
    {
        // Candidate only: the API drops it when an instrumented service answered the call.
        let id = EntityId::derive_resource(scope, "external", "http", &host);
        return Some((id.to_string(), host, "external".into()));
    }
    None
}

/// Host (plus a non-default port) an HTTP client span talks to: `server.address`/`server.port`,
/// else the URL authority. Telemetry is untrusted, so anything that is not a plain host name,
/// IP or port number is ignored.
fn http_peer_host(attrs: &[(String, String)]) -> Option<String> {
    let url = attr(attrs, "url.full").or_else(|| attr(attrs, "http.url")).and_then(url_authority);
    let raw = attr(attrs, "server.address")
        .or_else(|| attr(attrs, "net.peer.name"))
        .map(str::to_owned)
        .or_else(|| url.as_ref().map(|(h, _)| h.clone()))?;
    let host = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'));
    if !valid {
        return None;
    }
    let port = attr(attrs, "server.port")
        .or_else(|| attr(attrs, "net.peer.port"))
        .map(str::to_owned)
        .or_else(|| url.and_then(|(_, p)| p))
        .and_then(|p| p.trim().parse::<u16>().ok())
        .filter(|p| !matches!(p, 0 | 80 | 443));
    Some(match port {
        Some(p) if host.contains(':') && !host.starts_with('[') => format!("[{host}]:{p}"),
        Some(p) => format!("{host}:{p}"),
        None => host,
    })
}

/// Host and port of an absolute URL, without credentials, path or query.
fn url_authority(url: &str) -> Option<(String, Option<String>)> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if host_port.starts_with('[') {
        let (h, tail) = host_port.split_once(']')?;
        (format!("{h}]"), tail.strip_prefix(':'))
    } else {
        match host_port.split_once(':') {
            Some((h, p)) => (h.to_owned(), Some(p)),
            None => (host_port.to_owned(), None),
        }
    };
    Some((host, port.map(str::to_owned)))
}

fn hash_field(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}

fn sorted_attr_refs(attrs: &[(String, String)]) -> Vec<&(String, String)> {
    let mut sorted: Vec<_> = attrs.iter().collect();
    sorted.sort();
    sorted
}

fn series_id_prefix(name: &str, sorted_resource: &[&(String, String)]) -> Sha256 {
    let mut h = Sha256::new();
    hash_field(&mut h, name.as_bytes());
    hash_field(&mut h, &(sorted_resource.len() as u64).to_le_bytes());
    for (key, value) in sorted_resource {
        hash_field(&mut h, key.as_bytes());
        hash_field(&mut h, value.as_bytes());
    }
    h
}

fn series_id_with_prefix(prefix: &Sha256, attributes: &[(String, String)]) -> u64 {
    let sorted_attributes = sorted_attr_refs(attributes);
    let mut h = prefix.clone();
    hash_field(&mut h, &(sorted_attributes.len() as u64).to_le_bytes());
    for (key, value) in sorted_attributes {
        hash_field(&mut h, key.as_bytes());
        hash_field(&mut h, value.as_bytes());
    }
    let digest = h.finalize();
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(first)
}

fn point_value(p: &NumberDataPoint) -> Option<f64> {
    match p.value? {
        number_data_point::Value::AsDouble(v) => Some(v).filter(|v| v.is_finite()),
        number_data_point::Value::AsInt(v) => Some(v as f64),
    }
}

fn convert_metrics(req: ExportMetricsServiceRequest, base: &MetricRow, out: &mut Converted) {
    for rm in req.resource_metrics {
        let resource = rm.resource.map(|r| flatten(&r.attributes)).unwrap_or_default();
        let sorted_resource = sorted_attr_refs(&resource);
        let get = |k: &str| attr(&resource, k).unwrap_or("").to_owned();
        let host_name = get("host.name");
        let host_id = attr(&resource, "host.id").map_or_else(|| host_name.clone(), str::to_owned);
        let template = MetricRow {
            host_id,
            host_name,
            container_id: get("container.id"),
            container_name: get("container.name"),
            container_hostname: get("container.hostname"),
            container_image: get("container.image.name"),
            container_service: get("docker.compose.service"),
            service_name: get("service.name"),
            ..base.clone()
        };
        for sm in rm.scope_metrics {
            for m in sm.metrics {
                let (kind, points) = match m.data {
                    Some(Data::Gauge(g)) => (METRIC_KIND_GAUGE, g.data_points),
                    Some(Data::Sum(s)) if s.aggregation_temporality == 1 => (METRIC_KIND_DELTA_SUM, s.data_points),
                    Some(Data::Sum(s)) => (METRIC_KIND_CUMULATIVE_SUM, s.data_points),
                    Some(Data::Histogram(h)) => {
                        let kind = match h.aggregation_temporality {
                            1 => METRIC_KIND_DELTA_SUM,
                            2 => METRIC_KIND_CUMULATIVE_SUM,
                            _ => {
                                out.dropped_points += h.data_points.len() as u64;
                                continue;
                            }
                        };
                        for p in h.data_points {
                            let valid = p.time_unix_nano > 0
                                && !p.bucket_counts.is_empty()
                                && p.bucket_counts.len() <= 65
                                && p.bucket_counts.len() == p.explicit_bounds.len() + 1
                                && p.explicit_bounds.iter().all(|b| b.is_finite())
                                && p.explicit_bounds.windows(2).all(|pair| pair[0] < pair[1])
                                && p.bucket_counts.iter().try_fold(0u64, |sum, count| sum.checked_add(*count))
                                    == Some(p.count)
                                && p.sum.is_none_or(f64::is_finite);
                            if !valid {
                                out.dropped_points += 1;
                                continue;
                            }
                            let mut attributes = flatten(&p.attributes);
                            // The bucket boundary is assigned by the converter, never by source attributes.
                            attributes.retain(|(key, _)| key != "le");
                            let time = clamp_ns(p.time_unix_nano);
                            let mut emit = |suffix: &str, attrs: Vec<(String, String)>, value: f64| {
                                let metric_name = format!("{}.{}", m.name, suffix);
                                let prefix = series_id_prefix(&metric_name, &sorted_resource);
                                out.metrics.push(MetricRow {
                                    metric_name,
                                    kind,
                                    unit: if suffix == "count" { "1".into() } else { m.unit.clone() },
                                    series_id: series_id_with_prefix(&prefix, &attrs),
                                    attributes: attrs,
                                    value,
                                    time,
                                    ..template.clone()
                                });
                            };
                            for (index, count) in p.bucket_counts.iter().enumerate() {
                                let upper =
                                    p.explicit_bounds.get(index).map_or_else(|| "+Inf".into(), ToString::to_string);
                                let mut attrs = attributes.clone();
                                attrs.push(("le".into(), upper));
                                emit("bucket", attrs, *count as f64);
                            }
                            emit("count", attributes.clone(), p.count as f64);
                            if let Some(sum) = p.sum {
                                emit("sum", attributes, sum);
                            }
                        }
                        continue;
                    }
                    Some(Data::ExponentialHistogram(h)) => {
                        out.dropped_points += h.data_points.len() as u64;
                        continue;
                    }
                    Some(Data::Summary(s)) => {
                        out.dropped_points += s.data_points.len() as u64;
                        continue;
                    }
                    None => continue,
                };
                let id_prefix = series_id_prefix(&m.name, &sorted_resource);
                for p in points {
                    let Some(value) = point_value(&p).filter(|_| p.time_unix_nano > 0) else {
                        out.dropped_points += 1;
                        continue;
                    };
                    let attributes = flatten(&p.attributes);
                    out.metrics.push(MetricRow {
                        metric_name: m.name.clone(),
                        kind,
                        unit: m.unit.clone(),
                        series_id: series_id_with_prefix(&id_prefix, &attributes),
                        attributes,
                        value,
                        time: clamp_ns(p.time_unix_nano),
                        ..template.clone()
                    });
                }
            }
        }
    }
}

pub fn scope_of(env: &IngestEnvelope) -> Result<Scope, ConvertError> {
    let s = env.scope.as_ref().ok_or_else(|| ConvertError::Scope("missing".into()))?;
    let err = |e: karbird_domain::IdError| ConvertError::Scope(e.to_string());
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
            for rl in req.resource_logs {
                let resource = rl.resource.map(|r| flatten(&r.attributes)).unwrap_or_default();
                let svc = service_identity(&scope, &resource);
                for sl in rl.scope_logs {
                    for lr in sl.log_records {
                        let time = if lr.time_unix_nano > 0 { lr.time_unix_nano } else { lr.observed_time_unix_nano };
                        let body = lr.body.as_ref().map(any_to_string).unwrap_or_default();
                        let attributes = flatten(&lr.attributes);
                        let log_id = log_fingerprint(&svc, time, &lr, &body, &attributes);
                        out.logs.push(LogRow {
                            tenant_id: t.clone(),
                            project_id: p.clone(),
                            environment_id: e.clone(),
                            service_id: svc.id.clone(),
                            service_name: svc.name.clone(),
                            service_instance_id: svc.instance.clone(),
                            log_id,
                            time: clamp_ns(time),
                            observed_time: clamp_ns(lr.observed_time_unix_nano),
                            severity_number: u8::try_from(lr.severity_number).unwrap_or(0),
                            severity_text: lr.severity_text,
                            template_id: karbird_archive::template_id(&karbird_archive::parse(&body).template),
                            body,
                            trace_id: hex::encode(&lr.trace_id),
                            span_id: hex::encode(&lr.span_id),
                            attributes,
                            resource_attributes: resource.clone(),
                            batch_id: env.batch_id.clone(),
                            ingested_at,
                        });
                    }
                }
            }
        }
        Payload::OtlpMetrics(bytes) => {
            let req = ExportMetricsServiceRequest::decode(bytes.as_slice())
                .map_err(|e| ConvertError::Decode(e.to_string()))?;
            let base = MetricRow {
                tenant_id: t.clone(),
                project_id: p.clone(),
                environment_id: e.clone(),
                metric_name: String::new(),
                kind: 0,
                unit: String::new(),
                series_id: 0,
                host_id: String::new(),
                host_name: String::new(),
                container_id: String::new(),
                container_name: String::new(),
                container_hostname: String::new(),
                container_image: String::new(),
                container_service: String::new(),
                service_name: String::new(),
                attributes: Vec::new(),
                value: 0.0,
                time: 0,
                batch_id: env.batch_id.clone(),
                ingested_at,
            };
            convert_metrics(req, &base, &mut out);
        }
        Payload::CustomEvents(batch) => {
            out.events.reserve(batch.events.len());
            let mut service_ids = HashMap::<(&str, &str), String>::new();
            for ev in &batch.events {
                let namespace = ev.service_namespace.as_str();
                let name = ev.service_name.as_str();
                let service_id = service_ids
                    .entry((namespace, name))
                    .or_insert_with(|| ServiceId::derive(&scope, namespace, name).to_string())
                    .clone();
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
    use karbird_protocol::otlp::resource::v1::Resource;
    use karbird_protocol::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use karbird_protocol::v1::IngestScope;

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
            envelope("t1", vec![kv("service.name", "orders"), kv("tenant_id", "t2"), kv("karbird.tenant_id", "t2")]);
        let out = convert(&env, 5).unwrap();
        assert_eq!(out.spans[0].tenant_id, "t1");
        let other = convert(&envelope("t2", vec![kv("service.name", "orders")]), 5).unwrap();
        assert_ne!(out.spans[0].service_id, other.spans[0].service_id);
    }

    fn log_envelope(batch_id: &str, body: &str) -> IngestEnvelope {
        use karbird_protocol::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource { attributes: vec![kv("service.name", "orders")], ..Default::default() }),
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        time_unix_nano: 1_000,
                        severity_number: 9,
                        body: Some(AnyValue { value: Some(Value::StringValue(body.into())) }),
                        attributes: vec![kv("order.id", "7"), kv("http.route", "/orders/:id")],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let mut env = envelope("t1", vec![]);
        env.batch_id = batch_id.into();
        env.payload = Some(Payload::OtlpLogs(req.encode_to_vec()));
        env
    }

    #[test]
    fn retried_log_export_gets_the_same_log_id() {
        let first = convert(&log_envelope("batch-a", "order loaded"), 1).unwrap();
        let retry = convert(&log_envelope("batch-b", "order loaded"), 2).unwrap();
        let other = convert(&log_envelope("batch-a", "order created"), 1).unwrap();
        assert_eq!(first.logs[0].log_id, retry.logs[0].log_id);
        assert_ne!(first.logs[0].log_id, other.logs[0].log_id);
        assert_eq!(first.logs[0].log_id.len(), 32);
    }

    #[test]
    fn logs_of_one_kind_share_a_template_id() {
        let id = |body: &str| convert(&log_envelope("b", body), 1).unwrap().logs[0].template_id;
        assert_eq!(id("order 81723 created in 12 ms"), id("order 81724 created in 9 ms"));
        assert_ne!(id("order 81723 created in 12 ms"), id("payment failed for order 81723"));
        assert_ne!(id(""), 0, "even an empty body has a template");
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let mut env = envelope("t1", vec![]);
        env.schema_version = 99;
        assert!(matches!(convert(&env, 1), Err(ConvertError::UnsupportedVersion(99))));
    }

    use karbird_protocol::otlp::common::v1::{ArrayValue, KeyValueList};
    use karbird_protocol::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use karbird_protocol::otlp::trace::v1::Status;
    use karbird_protocol::v1::{CustomEvent, CustomEventBatch};

    fn anyv(v: Value) -> AnyValue {
        AnyValue { value: Some(v) }
    }

    fn traces_env(resource_attrs: Vec<KeyValue>, spans: Vec<Span>) -> IngestEnvelope {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource { attributes: resource_attrs, ..Default::default() }),
                scope_spans: vec![ScopeSpans { spans, ..Default::default() }],
                ..Default::default()
            }],
        };
        let mut env = envelope("t1", vec![]);
        env.payload = Some(Payload::OtlpTraces(req.encode_to_vec()));
        env
    }

    fn logs_env(resource_attrs: Vec<KeyValue>, records: Vec<LogRecord>) -> IngestEnvelope {
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource { attributes: resource_attrs, ..Default::default() }),
                scope_logs: vec![ScopeLogs { log_records: records, ..Default::default() }],
                ..Default::default()
            }],
        };
        let mut env = envelope("t1", vec![]);
        env.payload = Some(Payload::OtlpLogs(req.encode_to_vec()));
        env
    }

    fn events_env(events: Vec<CustomEvent>) -> IngestEnvelope {
        let mut env = envelope("t1", vec![]);
        env.received_time_unix_nano = 777;
        env.payload = Some(Payload::CustomEvents(CustomEventBatch { events }));
        env
    }

    fn scope() -> Scope {
        Scope {
            tenant_id: TenantId::parse("t1").unwrap(),
            project_id: ProjectId::parse("p1").unwrap(),
            environment_id: EnvironmentId::parse("dev").unwrap(),
        }
    }

    #[test]
    fn span_fields_are_mapped_one_to_one() {
        let span = Span {
            trace_id: vec![0xab; 16],
            span_id: vec![0xcd; 8],
            parent_span_id: vec![0xef; 8],
            kind: 2,
            name: "GET /orders".into(),
            start_time_unix_nano: 1_000,
            end_time_unix_nano: 4_500,
            status: Some(Status { code: 2, message: "boom".into() }),
            attributes: vec![kv("http.route", "/orders"), kv("http.response.status_code", "500")],
            ..Default::default()
        };
        let resource = vec![
            kv("service.name", "orders"),
            kv("service.namespace", "shop"),
            kv("service.instance.id", "pod-1"),
            kv("service.version", "1.2.3"),
        ];
        let mut env = traces_env(resource.clone(), vec![span]);
        env.batch_id = "batch-9".into();
        let out = convert(&env, 42).unwrap();
        assert!(out.logs.is_empty() && out.events.is_empty());
        let s = &out.spans[0];
        assert_eq!((s.tenant_id.as_str(), s.project_id.as_str(), s.environment_id.as_str()), ("t1", "p1", "dev"));
        assert_eq!(s.service_id, ServiceId::derive(&scope(), "shop", "orders").to_string());
        assert_eq!(s.service_name, "orders");
        assert_eq!(s.service_namespace, "shop");
        assert_eq!(s.service_instance_id, "pod-1");
        assert_eq!(s.service_version, "1.2.3");
        assert_eq!(s.trace_id, "ab".repeat(16));
        assert_eq!(s.span_id, "cd".repeat(8));
        assert_eq!(s.parent_span_id, "ef".repeat(8));
        assert_eq!(s.name, "GET /orders");
        assert_eq!(s.kind, 2);
        assert_eq!((s.start_time, s.end_time, s.duration_ns), (1_000, 4_500, 3_500));
        assert_eq!(s.status_code, 2);
        assert_eq!(s.status_message, "boom");
        assert!(s.target_entity_id.is_empty() && s.target_name.is_empty() && s.target_kind.is_empty());
        assert_eq!(
            s.attributes,
            vec![("http.route".into(), "/orders".into()), ("http.response.status_code".into(), "500".into())]
        );
        assert_eq!(s.resource_attributes.len(), 4);
        assert_eq!(s.batch_id, "batch-9");
        assert_eq!(s.ingested_at, 42);
    }

    #[test]
    fn missing_resource_and_status_use_safe_defaults() {
        let req = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: None,
                scope_spans: vec![ScopeSpans {
                    spans: vec![Span { trace_id: vec![1; 16], span_id: vec![2; 8], ..Default::default() }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let mut env = envelope("t1", vec![]);
        env.payload = Some(Payload::OtlpTraces(req.encode_to_vec()));
        let s = &convert(&env, 1).unwrap().spans[0];
        assert_eq!(s.service_name, "unknown_service");
        assert_eq!(s.service_id, ServiceId::derive(&scope(), "", "unknown_service").to_string());
        assert!(s.parent_span_id.is_empty());
        assert_eq!((s.status_code, s.status_message.as_str()), (0, ""));
        assert!(s.resource_attributes.is_empty());
    }

    #[test]
    fn empty_service_name_falls_back_to_unknown() {
        let env = traces_env(vec![kv("service.name", "")], vec![Span::default()]);
        assert_eq!(convert(&env, 1).unwrap().spans[0].service_name, "unknown_service");
    }

    #[test]
    fn out_of_range_numbers_are_clamped_not_wrapped() {
        let span = Span {
            kind: 300,
            start_time_unix_nano: u64::MAX,
            end_time_unix_nano: u64::MAX,
            status: Some(Status { code: -1, message: String::new() }),
            ..Default::default()
        };
        let s = &convert(&traces_env(vec![], vec![span]), 1).unwrap().spans[0];
        assert_eq!(s.kind, 0);
        assert_eq!(s.status_code, 0);
        assert_eq!((s.start_time, s.end_time, s.duration_ns), (i64::MAX, i64::MAX, 0));
        assert_eq!(clamp_ns(0), 0);
        assert_eq!(clamp_ns(i64::MAX as u64), i64::MAX);
        assert_eq!(clamp_ns(i64::MAX as u64 + 1), i64::MAX);
    }

    #[test]
    fn inverted_span_times_do_not_panic() {
        let span = Span { start_time_unix_nano: 100, end_time_unix_nano: 50, ..Default::default() };
        let s = &convert(&traces_env(vec![], vec![span]), 1).unwrap().spans[0];
        assert_eq!(s.duration_ns, -50);
    }

    #[test]
    fn span_targets_follow_semantic_conventions() {
        let s = scope();
        let attrs = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
        };
        // Only client (3) and producer (4) spans have targets.
        for kind in [0, 1, 2, 5] {
            assert!(span_target(&s, kind, &attrs(&[("db.system", "postgresql")])).is_none());
        }
        let (id, name, kind) = span_target(
            &s,
            3,
            &attrs(&[("db.system.name", "postgresql"), ("db.system", "mysql"), ("db.namespace", "orders")]),
        )
        .unwrap();
        assert_eq!((name.as_str(), kind.as_str()), ("postgresql: orders", "database"));
        assert_eq!(id, EntityId::derive_resource(&s, "database", "postgresql", "orders").to_string());
        let name_of = |pairs: &[(&str, &str)]| span_target(&s, 3, &attrs(pairs)).map(|t| t.1);
        assert_eq!(
            name_of(&[("db.system", "redis"), ("server.address", "cache-1")]).as_deref(),
            Some("redis: cache-1")
        );
        assert_eq!(name_of(&[("db.system", "redis"), ("net.peer.name", "cache-2")]).as_deref(), Some("redis: cache-2"));
        assert_eq!(name_of(&[("db.system", "redis")]).as_deref(), Some("redis: redis"));
        assert_eq!(name_of(&[("db.system", ""), ("db.name", "x")]), None, "empty system is ignored");
        let (_, name, kind) = span_target(
            &s,
            4,
            &attrs(&[("messaging.system", "kafka"), ("messaging.destination.name", "orders.created")]),
        )
        .unwrap();
        assert_eq!((name.as_str(), kind.as_str()), ("kafka: orders.created", "queue"));
        assert_eq!(
            span_target(&s, 4, &attrs(&[("messaging.system", "rabbitmq")])).map(|t| t.1).as_deref(),
            Some("rabbitmq: rabbitmq")
        );
        assert!(
            span_target(&s, 3, &attrs(&[("http.url", "https://x")])).is_none(),
            "a URL without an HTTP method is not treated as an HTTP call"
        );
    }

    #[test]
    fn http_client_spans_target_the_host_they_call() {
        let s = scope();
        let attrs = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
        };
        let target = |kind: i32, pairs: &[(&str, &str)]| span_target(&s, kind, &attrs(pairs));

        let (id, name, kind) =
            target(3, &[("http.request.method", "POST"), ("server.address", "API.CDEK.ru."), ("server.port", "443")])
                .unwrap();
        assert_eq!((name.as_str(), kind.as_str()), ("api.cdek.ru", "external"));
        assert_eq!(id, EntityId::derive_resource(&s, "external", "http", "api.cdek.ru").to_string());

        let host = |pairs: &[(&str, &str)]| target(3, pairs).map(|t| t.1);
        assert_eq!(host(&[("http.method", "GET"), ("net.peer.name", "b.example")]).as_deref(), Some("b.example"));
        assert_eq!(
            host(&[("http.request.method", "GET"), ("url.full", "https://user:pw@pay.example/v1?token=x")]).as_deref(),
            Some("pay.example"),
            "credentials, path and query never become part of the name"
        );
        // A non-default port tells apart services on one host (e.g. everything on 127.0.0.1 locally).
        assert_eq!(
            host(&[("http.request.method", "GET"), ("url.full", "https://pay.example:8443/v1?token=x")]).as_deref(),
            Some("pay.example:8443")
        );
        assert_eq!(
            host(&[("http.request.method", "GET"), ("server.address", "127.0.0.1"), ("server.port", "7010")])
                .as_deref(),
            Some("127.0.0.1:7010")
        );
        assert_eq!(
            host(&[("http.method", "GET"), ("net.peer.name", "b"), ("net.peer.port", "80")]).as_deref(),
            Some("b")
        );
        assert_eq!(host(&[("http.method", "GET"), ("http.url", "http://[::1]:9000/x")]).as_deref(), Some("[::1]:9000"));
        assert_eq!(
            host(&[("http.method", "GET"), ("server.address", "::1"), ("server.port", "9000")]).as_deref(),
            Some("[::1]:9000"),
            "a bare IPv6 address is bracketed before the port is added"
        );
        for bad in ["0", "65536", "-1", "80a", ""] {
            assert_eq!(
                host(&[("http.method", "GET"), ("server.address", "c.example"), ("server.port", bad)]).as_deref(),
                Some("c.example"),
                "invalid port {bad:?} is ignored"
            );
        }
        assert_ne!(
            target(3, &[("http.method", "GET"), ("server.address", "127.0.0.1"), ("server.port", "7010")]).map(|t| t.0),
            target(3, &[("http.method", "GET"), ("server.address", "127.0.0.1"), ("server.port", "7011")]).map(|t| t.0),
        );
        assert_eq!(
            host(&[
                ("http.request.method", "GET"),
                ("server.address", "a.example"),
                ("url.full", "https://b.example/")
            ])
            .as_deref(),
            Some("a.example"),
            "server.address wins over the URL"
        );

        // Server, internal and producer spans are not outgoing HTTP calls.
        for kind in [1, 2, 4, 5] {
            assert!(target(kind, &[("http.request.method", "GET"), ("server.address", "a.example")]).is_none());
        }
        // No host, or a host that is not a plain name/IP (untrusted input), gives no target.
        assert!(target(3, &[("http.request.method", "GET")]).is_none());
        assert!(target(3, &[("http.request.method", "GET"), ("url.full", "/relative/path")]).is_none());
        assert!(target(3, &[("http.request.method", "GET"), ("server.address", "<script>")]).is_none());
        assert!(target(3, &[("http.request.method", "GET"), ("server.address", "a b")]).is_none());
        assert!(target(3, &[("http.request.method", "GET"), ("server.address", &"a".repeat(254))]).is_none());
        // Database attributes take precedence over HTTP ones.
        assert_eq!(
            target(3, &[("db.system", "postgresql"), ("http.request.method", "GET"), ("server.address", "db1")])
                .map(|t| t.2)
                .as_deref(),
            Some("database")
        );
    }

    #[test]
    fn any_values_are_stringified_deterministically() {
        assert_eq!(any_to_string(&anyv(Value::StringValue("s".into()))), "s");
        assert_eq!(any_to_string(&anyv(Value::BoolValue(false))), "false");
        assert_eq!(any_to_string(&anyv(Value::IntValue(-7))), "-7");
        assert_eq!(any_to_string(&anyv(Value::DoubleValue(1.5))), "1.5");
        assert_eq!(any_to_string(&anyv(Value::BytesValue(vec![0xde, 0xad]))), "dead");
        let arr = anyv(Value::ArrayValue(ArrayValue {
            values: vec![anyv(Value::IntValue(1)), anyv(Value::StringValue("a\"b".into()))],
        }));
        assert_eq!(any_to_string(&arr), r#"["1","a\"b"]"#);
        let kvl = anyv(Value::KvlistValue(KeyValueList {
            values: vec![kv("k", "v"), KeyValue { key: "none".into(), value: None, ..Default::default() }],
        }));
        assert_eq!(any_to_string(&kvl), r#"{"k":"v","none":""}"#);
        assert_eq!(any_to_string(&AnyValue { value: None }), "");
    }

    #[test]
    fn attributes_without_values_become_empty_strings() {
        let flat = flatten(&[KeyValue { key: "k".into(), value: None, ..Default::default() }]);
        assert_eq!(flat, vec![("k".to_owned(), String::new())]);
        assert_eq!(attr(&flat, "k"), None, "empty values are treated as missing");
        assert_eq!(attr(&flat, "missing"), None);
    }

    #[test]
    fn log_fields_are_mapped() {
        let lr = LogRecord {
            time_unix_nano: 2_000,
            observed_time_unix_nano: 2_100,
            severity_number: 17,
            severity_text: "ERROR".into(),
            body: Some(anyv(Value::StringValue("payment declined".into()))),
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            attributes: vec![kv("order.id", "7")],
            ..Default::default()
        };
        let out =
            convert(&logs_env(vec![kv("service.name", "payments"), kv("service.instance.id", "i-1")], vec![lr]), 9)
                .unwrap();
        assert!(out.spans.is_empty() && out.events.is_empty());
        let l = &out.logs[0];
        assert_eq!(l.service_name, "payments");
        assert_eq!(l.service_instance_id, "i-1");
        assert_eq!((l.time, l.observed_time), (2_000, 2_100));
        assert_eq!(l.severity_number, 17);
        assert_eq!(l.severity_text, "ERROR");
        assert_eq!(l.body, "payment declined");
        assert_eq!(l.trace_id, "01".repeat(16));
        assert_eq!(l.span_id, "02".repeat(8));
        assert_eq!(l.attributes, vec![("order.id".into(), "7".into())]);
        assert_eq!(l.ingested_at, 9);
        assert_eq!(l.tenant_id, "t1");
    }

    #[test]
    fn log_time_falls_back_to_observed_time() {
        let lr = LogRecord { time_unix_nano: 0, observed_time_unix_nano: 5_000, ..Default::default() };
        let l = &convert(&logs_env(vec![], vec![lr]), 1).unwrap().logs[0];
        assert_eq!((l.time, l.observed_time), (5_000, 5_000));
        assert_eq!(l.service_name, "unknown_service");
        assert!(l.body.is_empty() && l.trace_id.is_empty());
        let big = LogRecord { time_unix_nano: u64::MAX, severity_number: 1_000, ..Default::default() };
        let l = &convert(&logs_env(vec![], vec![big]), 1).unwrap().logs[0];
        assert_eq!(l.time, i64::MAX);
        assert_eq!(l.severity_number, 0);
    }

    #[test]
    fn structured_log_bodies_are_stringified() {
        let lr = LogRecord {
            time_unix_nano: 1,
            body: Some(anyv(Value::KvlistValue(KeyValueList { values: vec![kv("msg", "hi")] }))),
            ..Default::default()
        };
        assert_eq!(convert(&logs_env(vec![], vec![lr]), 1).unwrap().logs[0].body, r#"{"msg":"hi"}"#);
    }

    #[test]
    fn log_ids_ignore_attribute_order_but_not_content() {
        let rec = |attrs: Vec<KeyValue>, severity: i32| LogRecord {
            time_unix_nano: 1,
            severity_number: severity,
            attributes: attrs,
            ..Default::default()
        };
        let id = |r: LogRecord, instance: &str| {
            convert(&logs_env(vec![kv("service.name", "s"), kv("service.instance.id", instance)], vec![r]), 1)
                .unwrap()
                .logs[0]
                .log_id
                .clone()
        };
        let a = id(rec(vec![kv("a", "1"), kv("b", "2")], 9), "i1");
        assert_eq!(a, id(rec(vec![kv("b", "2"), kv("a", "1")], 9), "i1"));
        assert_ne!(a, id(rec(vec![kv("a", "1"), kv("b", "3")], 9), "i1"));
        assert_ne!(a, id(rec(vec![kv("a", "1"), kv("b", "2")], 10), "i1"));
        assert_ne!(a, id(rec(vec![kv("a", "1"), kv("b", "2")], 9), "i2"));
        // Field boundaries are length-prefixed: ("ab","") != ("a","b").
        assert_ne!(id(rec(vec![kv("ab", "")], 9), "i1"), id(rec(vec![kv("a", "b")], 9), "i1"));
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn log_ids_differ_across_tenants() {
        let lr = || LogRecord { time_unix_nano: 1, ..Default::default() };
        let a = convert(&logs_env(vec![kv("service.name", "s")], vec![lr()]), 1).unwrap();
        let mut other = logs_env(vec![kv("service.name", "s")], vec![lr()]);
        other.scope.as_mut().unwrap().tenant_id = "t2".into();
        let b = convert(&other, 1).unwrap();
        assert_ne!(a.logs[0].log_id, b.logs[0].log_id);
    }

    #[test]
    fn event_fields_are_mapped() {
        let mut ev = CustomEvent {
            event_id: "e1".into(),
            event_type: "deployment.started".into(),
            service_name: "orders".into(),
            service_namespace: "shop".into(),
            service_instance_id: "pod-1".into(),
            entity_id: "svc_x".into(),
            event_time_unix_nano: 1_000,
            observed_time_unix_nano: 1_100,
            trace_id: "ab".repeat(16),
            span_id: "cd".repeat(8),
            source_id: "sdk".into(),
            source_epoch: "boot".into(),
            source_sequence: Some(3),
            payload_json: "{}".into(),
            ..Default::default()
        };
        ev.attributes.insert("z".into(), "1".into());
        ev.attributes.insert("a".into(), "2".into());
        let out = convert(&events_env(vec![ev]), 5).unwrap();
        assert!(out.spans.is_empty() && out.logs.is_empty());
        let e = &out.events[0];
        assert_eq!(e.event_id, "e1");
        assert_eq!(e.event_type, "deployment.started");
        assert_eq!(e.service_id, ServiceId::derive(&scope(), "shop", "orders").to_string());
        assert_eq!((e.service_name.as_str(), e.service_instance_id.as_str()), ("orders", "pod-1"));
        assert_eq!(e.entity_id, "svc_x");
        assert_eq!((e.event_time, e.observed_time), (1_000, 1_100));
        assert_eq!((e.trace_id.len(), e.span_id.len()), (32, 16));
        assert_eq!((e.source_id.as_str(), e.source_epoch.as_str(), e.source_sequence), ("sdk", "boot", Some(3)));
        assert_eq!(e.attributes, vec![("a".into(), "2".into()), ("z".into(), "1".into())], "sorted");
        assert_eq!(e.payload_json, "{}");
        assert_eq!(e.ingested_at, 5);
    }

    #[test]
    fn custom_events_keep_service_identity_when_services_repeat_interleaved() {
        let event = |id: &str, service: &str| CustomEvent {
            event_id: id.into(),
            service_namespace: "shop".into(),
            service_name: service.into(),
            ..Default::default()
        };
        let converted =
            convert(&events_env(vec![event("a", "orders"), event("b", "payments"), event("c", "orders")]), 1).unwrap();

        assert_eq!(converted.events[0].service_id, ServiceId::derive(&scope(), "shop", "orders").to_string());
        assert_eq!(converted.events[1].service_id, ServiceId::derive(&scope(), "shop", "payments").to_string());
        assert_eq!(converted.events[2].service_id, converted.events[0].service_id);
    }

    #[test]
    fn event_observed_time_defaults_to_receive_time() {
        let ev = CustomEvent { event_id: "e".into(), event_time_unix_nano: 1, ..Default::default() };
        assert_eq!(convert(&events_env(vec![ev]), 1).unwrap().events[0].observed_time, 777);
    }

    #[test]
    fn empty_batches_convert_to_nothing() {
        assert!(convert(&events_env(vec![]), 1).unwrap().is_empty());
        assert!(convert(&traces_env(vec![], vec![]), 1).unwrap().is_empty());
        assert!(convert(&logs_env(vec![], vec![]), 1).unwrap().is_empty());
        let mut env = envelope("t1", vec![]);
        env.payload = Some(Payload::OtlpTraces(vec![]));
        assert!(convert(&env, 1).unwrap().is_empty());
    }

    #[test]
    fn envelope_errors_are_classified() {
        let mut env = envelope("t1", vec![]);
        env.schema_version = 0;
        assert_eq!(convert(&env, 1).unwrap_err().kind(), "unsupported_version");

        let mut env = envelope("t1", vec![]);
        env.scope = None;
        assert_eq!(convert(&env, 1).unwrap_err().kind(), "invalid_scope");

        for bad in ["", "a b", "t'; DROP TABLE spans"] {
            let env = envelope(bad, vec![]);
            assert_eq!(convert(&env, 1).unwrap_err().kind(), "invalid_scope", "{bad:?}");
        }
        let mut env = envelope("t1", vec![]);
        env.scope.as_mut().unwrap().environment_id = String::new();
        assert!(matches!(convert(&env, 1), Err(ConvertError::Scope(_))));

        let mut env = envelope("t1", vec![]);
        env.payload = None;
        assert_eq!(convert(&env, 1).unwrap_err().kind(), "empty_payload");

        let mut env = envelope("t1", vec![]);
        env.payload = Some(Payload::OtlpTraces(vec![0xff, 0xff]));
        assert_eq!(convert(&env, 1).unwrap_err().kind(), "decode");
        env.payload = Some(Payload::OtlpLogs(vec![0x0a, 0x05]));
        assert_eq!(convert(&env, 1).unwrap_err().kind(), "decode");
    }

    #[test]
    fn decode_envelope_rejects_garbage() {
        assert!(matches!(decode_envelope(&[0x0f]), Err(ConvertError::Decode(_))));
        let env = envelope("t1", vec![]);
        assert_eq!(decode_envelope(&env.encode_to_vec()).unwrap(), env);
        // An empty record decodes to a default envelope, which convert then rejects.
        let empty = decode_envelope(&[]).unwrap();
        assert!(matches!(convert(&empty, 1), Err(ConvertError::UnsupportedVersion(0))));
    }

    #[test]
    fn converted_extend_and_is_empty() {
        let mut a = Converted::default();
        assert!(a.is_empty());
        a.extend(convert(&envelope("t1", vec![]), 1).unwrap());
        a.extend(convert(&log_envelope("b", "x"), 1).unwrap());
        a.extend(Converted::default());
        assert!(!a.is_empty());
        assert_eq!((a.spans.len(), a.logs.len(), a.events.len()), (1, 1, 0));
    }

    use karbird_protocol::otlp::metrics::v1::{
        Gauge, Histogram, HistogramDataPoint, Metric, ResourceMetrics, ScopeMetrics, Sum,
    };

    fn num(time: u64, v: number_data_point::Value, attrs: Vec<KeyValue>) -> NumberDataPoint {
        NumberDataPoint { attributes: attrs, time_unix_nano: time, value: Some(v), ..Default::default() }
    }

    fn metrics_env(tenant: &str, resource_attrs: Vec<KeyValue>, metrics: Vec<Metric>) -> IngestEnvelope {
        let req = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource { attributes: resource_attrs, ..Default::default() }),
                scope_metrics: vec![ScopeMetrics { metrics, ..Default::default() }],
                ..Default::default()
            }],
        };
        let mut env = envelope(tenant, vec![]);
        env.batch_id = "mb".into();
        env.payload = Some(Payload::OtlpMetrics(req.encode_to_vec()));
        env
    }

    fn cpu_time(points: Vec<NumberDataPoint>, temporality: i32) -> Metric {
        Metric {
            name: "system.cpu.time".into(),
            unit: "s".into(),
            data: Some(Data::Sum(Sum {
                data_points: points,
                aggregation_temporality: temporality,
                is_monotonic: true,
            })),
            ..Default::default()
        }
    }

    #[test]
    fn metric_points_become_rows_with_host_and_container_columns() {
        use number_data_point::Value::{AsDouble, AsInt};
        let env = metrics_env(
            "t1",
            vec![
                kv("host.name", "vps-app"),
                kv("container.id", "abc123"),
                kv("container.name", "orders"),
                kv("container.hostname", "abc123def456"),
                kv("container.image.name", "postgres:16-alpine"),
                kv("docker.compose.service", "db"),
                kv("service.name", "orders"),
            ],
            vec![
                cpu_time(vec![num(10, AsDouble(1.5), vec![kv("cpu", "cpu0"), kv("state", "user")])], 2),
                Metric {
                    name: "system.memory.usage".into(),
                    unit: "By".into(),
                    data: Some(Data::Gauge(Gauge { data_points: vec![num(11, AsInt(870 << 20), vec![])] })),
                    ..Default::default()
                },
                cpu_time(vec![num(12, AsDouble(0.5), vec![])], 1),
            ],
        );
        let out = convert(&env, 99).unwrap();
        assert_eq!(out.metrics.len(), 3);
        assert_eq!(out.dropped_points, 0);
        let m = &out.metrics[0];
        assert_eq!((m.tenant_id.as_str(), m.project_id.as_str(), m.environment_id.as_str()), ("t1", "p1", "dev"));
        assert_eq!(
            (m.metric_name.as_str(), m.kind, m.unit.as_str()),
            ("system.cpu.time", METRIC_KIND_CUMULATIVE_SUM, "s")
        );
        assert_eq!(
            (m.host_id.as_str(), m.host_name.as_str()),
            ("vps-app", "vps-app"),
            "host.name stands in for host.id"
        );
        assert_eq!((m.container_id.as_str(), m.container_name.as_str()), ("abc123", "orders"));
        assert_eq!(m.container_hostname, "abc123def456");
        assert_eq!((m.container_image.as_str(), m.container_service.as_str()), ("postgres:16-alpine", "db"));
        assert_eq!(m.service_name, "orders");
        assert_eq!(m.attributes, vec![("cpu".into(), "cpu0".into()), ("state".into(), "user".into())]);
        assert_eq!((m.value, m.time, m.ingested_at, m.batch_id.as_str()), (1.5, 10, 99, "mb"));
        assert_eq!(out.metrics[1].kind, METRIC_KIND_GAUGE);
        assert_eq!(out.metrics[1].value, (870u64 << 20) as f64);
        assert_eq!(out.metrics[2].kind, METRIC_KIND_DELTA_SUM);
    }

    #[test]
    fn histogram_becomes_bucket_count_and_sum_rows() {
        let histogram = Metric {
            name: "http.server.request.duration".into(),
            unit: "s".into(),
            data: Some(Data::Histogram(Histogram {
                aggregation_temporality: 1,
                data_points: vec![HistogramDataPoint {
                    time_unix_nano: 42,
                    count: 3,
                    sum: Some(0.7),
                    explicit_bounds: vec![0.1, 0.5],
                    bucket_counts: vec![1, 1, 1],
                    attributes: vec![kv("http.route", "/orders/{id}")],
                    ..Default::default()
                }],
            })),
            ..Default::default()
        };
        let out = convert(&metrics_env("t1", vec![kv("service.name", "orders")], vec![histogram]), 8).unwrap();
        assert_eq!(out.dropped_points, 0);
        assert_eq!(out.metrics.len(), 5);
        assert_eq!(out.metrics.iter().map(|r| r.value).collect::<Vec<_>>(), vec![1.0, 1.0, 1.0, 3.0, 0.7]);
        assert_eq!(out.metrics[0].metric_name, "http.server.request.duration.bucket");
        assert_eq!(
            out.metrics[0].attributes,
            vec![("http.route".into(), "/orders/{id}".into()), ("le".into(), "0.1".into())]
        );
        assert_eq!(out.metrics[2].attributes[1], ("le".into(), "+Inf".into()));
        assert_eq!(out.metrics[3].metric_name, "http.server.request.duration.count");
        assert_eq!(out.metrics[4].metric_name, "http.server.request.duration.sum");
        assert_eq!(out.metrics[0].kind, METRIC_KIND_DELTA_SUM);
    }

    #[test]
    fn host_id_wins_over_host_name() {
        let env = metrics_env(
            "t1",
            vec![kv("host.name", "vps-app"), kv("host.id", "i-0abc")],
            vec![cpu_time(vec![num(1, number_data_point::Value::AsInt(1), vec![])], 2)],
        );
        let m = &convert(&env, 1).unwrap().metrics[0];
        assert_eq!((m.host_id.as_str(), m.host_name.as_str()), ("i-0abc", "vps-app"));
    }

    #[test]
    fn series_ids_are_stable_and_distinguish_series() {
        let id = |res: Vec<KeyValue>, attrs: Vec<KeyValue>, tenant: &str| {
            let env =
                metrics_env(tenant, res, vec![cpu_time(vec![num(1, number_data_point::Value::AsInt(1), attrs)], 2)]);
            convert(&env, 1).unwrap().metrics[0].series_id
        };
        let a = id(vec![kv("host.name", "h"), kv("x", "1")], vec![kv("cpu", "0"), kv("state", "idle")], "t1");
        assert_eq!(a, id(vec![kv("x", "1"), kv("host.name", "h")], vec![kv("state", "idle"), kv("cpu", "0")], "t1"));
        assert_eq!(
            a,
            id(vec![kv("host.name", "h"), kv("x", "1")], vec![kv("cpu", "0"), kv("state", "idle")], "t2"),
            "the scope is in the sorting key, the series id need not repeat it"
        );
        assert_ne!(a, id(vec![kv("host.name", "h2"), kv("x", "1")], vec![kv("cpu", "0"), kv("state", "idle")], "t1"));
        assert_ne!(a, id(vec![kv("host.name", "h"), kv("x", "1")], vec![kv("cpu", "1"), kv("state", "idle")], "t1"));
        assert_ne!(id(vec![kv("a", "1")], vec![], "t1"), id(vec![], vec![kv("a", "1")], "t1"));
    }

    #[test]
    fn series_id_keeps_the_existing_digest_for_varied_attribute_counts() {
        fn reference_series_id(name: &str, resource: &[(String, String)], attributes: &[(String, String)]) -> u64 {
            let mut h = Sha256::new();
            let mut field = |bytes: &[u8]| {
                h.update((bytes.len() as u64).to_le_bytes());
                h.update(bytes);
            };
            field(name.as_bytes());
            for group in [resource, attributes] {
                let mut sorted: Vec<&(String, String)> = group.iter().collect();
                sorted.sort();
                field(&(sorted.len() as u64).to_le_bytes());
                for (key, value) in sorted {
                    field(key.as_bytes());
                    field(value.as_bytes());
                }
            }
            let digest = h.finalize();
            let mut first = [0u8; 8];
            first.copy_from_slice(&digest[..8]);
            u64::from_le_bytes(first)
        }

        for resource_len in [0, 1, 4, 8, 9, 64] {
            for point_len in [0, 1, 4, 8, 9, 64] {
                let resource = (0..resource_len).map(|i| (format!("r{}", i % 5), format!("v{i}"))).collect::<Vec<_>>();
                let attributes = (0..point_len).map(|i| (format!("p{}", i % 7), format!("v{i}"))).collect::<Vec<_>>();
                assert_eq!(
                    series_id_with_prefix(
                        &series_id_prefix("system.cpu.utilization", &sorted_attr_refs(&resource)),
                        &attributes,
                    ),
                    reference_series_id("system.cpu.utilization", &resource, &attributes),
                    "resource attrs: {resource_len}, point attrs: {point_len}"
                );
            }
        }
    }

    #[test]
    fn unsupported_metric_points_are_counted_not_stored() {
        let hist = Metric {
            name: "http.server.duration".into(),
            data: Some(Data::Histogram(Histogram {
                data_points: vec![HistogramDataPoint::default(); 2],
                aggregation_temporality: 2,
            })),
            ..Default::default()
        };
        let bad_points = cpu_time(
            vec![
                num(0, number_data_point::Value::AsDouble(1.0), vec![]),
                num(1, number_data_point::Value::AsDouble(f64::NAN), vec![]),
                NumberDataPoint { time_unix_nano: 1, ..Default::default() },
                num(1, number_data_point::Value::AsDouble(2.0), vec![]),
            ],
            2,
        );
        let out = convert(&metrics_env("t1", vec![], vec![hist, bad_points]), 1).unwrap();
        assert_eq!(out.metrics.len(), 1);
        assert_eq!(out.dropped_points, 2 + 3);
        let mut all = Converted::default();
        all.extend(out);
        assert_eq!(all.dropped_points, 5);
        assert!(!all.is_empty());
    }

    #[test]
    fn malformed_metrics_payload_is_a_decode_error() {
        let mut env = envelope("t1", vec![]);
        env.payload = Some(Payload::OtlpMetrics(vec![0xff, 0xff]));
        assert_eq!(convert(&env, 1).unwrap_err().kind(), "decode");
        env.payload = Some(Payload::OtlpMetrics(vec![]));
        assert!(convert(&env, 1).unwrap().is_empty());
    }

    #[test]
    fn error_messages_carry_context() {
        assert!(ConvertError::UnsupportedVersion(7).to_string().contains('7'));
        assert!(ConvertError::Scope("missing".into()).to_string().contains("missing"));
        assert_eq!(ConvertError::EmptyPayload.to_string(), "envelope has no payload");
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
                if let Ok(env) = decode_envelope(&bytes) {
                    let _ = convert(&env, 1);
                }
            }

            #[test]
            fn arbitrary_otlp_payload_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..256), which in 0u8..3) {
                let mut env = envelope("t1", vec![]);
                env.payload = Some(match which {
                    0 => Payload::OtlpLogs(bytes),
                    1 => Payload::OtlpTraces(bytes),
                    _ => Payload::OtlpMetrics(bytes),
                });
                let _ = convert(&env, 1);
            }

            #[test]
            fn span_times_are_never_negative_and_ids_are_hex(
                start in any::<u64>(), end in any::<u64>(), trace in prop::collection::vec(any::<u8>(), 0..20)
            ) {
                let span = Span { trace_id: trace.clone(), start_time_unix_nano: start, end_time_unix_nano: end, ..Default::default() };
                let out = convert(&traces_env(vec![], vec![span]), 1).unwrap();
                let s = &out.spans[0];
                prop_assert!(s.start_time >= 0 && s.end_time >= 0);
                prop_assert_eq!(s.trace_id.len(), trace.len() * 2);
                prop_assert!(s.trace_id.bytes().all(|b| b.is_ascii_hexdigit()));
            }
        }
    }
}
