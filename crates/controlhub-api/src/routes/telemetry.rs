//! Scoped telemetry queries. Every handler resolves the scope through
//! [`authorize`] before touching storage.

use std::collections::{BTreeMap, HashSet};

use axum::Json;
use axum::extract::{Path, Query, State};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use controlhub_domain::{EntityId, Scope, TraceId};
use controlhub_engine_core::{
    EntityKindDto, EventItem, LogItem, ProvenanceDto, RelationKindDto, SpanItem, TimelineItem,
};
use controlhub_storage::ch::{
    CallEdgeRow, EventRow, ExternalClientEdgeRow, LogQuery, LogRow, LogServiceRow, ResourceEdgeRow, ServiceSummaryRow,
    SpanRow, TimelineLogRow, TimelineSpanRow, TraceQuery, TraceSummaryRow,
};
use controlhub_storage::pg::WatermarkRow;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth::{CurrentUser, Role, authorize};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const NS_PER_MIN: i64 = 60_000_000_000;
const MAX_RANGE_NS: i64 = 7 * 24 * 60 * NS_PER_MIN;
const PROCESSOR: &str = "controlhub-processor";
const TOPIC: &str = "controlhub.telemetry.v1";

pub(crate) fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
}

fn ns(v: i64) -> String {
    v.to_string()
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RangeParams {
    /// Start, unix epoch nanoseconds as a decimal string. Default: `to - 15m`.
    pub from: Option<String>,
    /// End (exclusive), unix epoch nanoseconds as a decimal string. Default: now.
    pub to: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Range {
    pub from: i64,
    pub to: i64,
}

pub(crate) fn parse_ns(v: &Option<String>, name: &str) -> ApiResult<Option<i64>> {
    v.as_ref()
        .map(|s| {
            s.parse::<i64>().map_err(|_| ApiError::BadRequest(format!("{name} must be a decimal nanosecond timestamp")))
        })
        .transpose()
}

impl RangeParams {
    pub fn resolve(&self) -> ApiResult<Range> {
        let to = parse_ns(&self.to, "to")?.unwrap_or_else(now_ns);
        let from = parse_ns(&self.from, "from")?.unwrap_or(to.saturating_sub(15 * NS_PER_MIN));
        if from >= to {
            return Err(ApiError::BadRequest("from must be before to".into()));
        }
        // Checked: extreme inputs must not wrap around into a "short" range.
        if to.checked_sub(from).is_none_or(|d| d > MAX_RANGE_NS) {
            return Err(ApiError::BadRequest("time range may not exceed 7 days".into()));
        }
        Ok(Range { from, to })
    }
}

fn limit(v: Option<u32>, default: u32, max: u32) -> u32 {
    v.unwrap_or(default).clamp(1, max)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Completeness {
    /// Data ingested before this time (nanoseconds) has been processed into storage.
    /// `null` when the processor has not reported progress yet.
    pub watermark_ns: Option<String>,
    pub processing_caught_up: bool,
    /// Records that failed processing and are waiting in the dead-letter table.
    pub unresolved_processing_errors: i64,
    /// The response hit a row limit; more data exists in the range.
    pub truncated: bool,
    /// Sampling configured in SDKs is not reported to ControlHub yet.
    pub sampling: String,
    pub notes: Vec<String>,
}

async fn completeness(state: &AppState, scope: &Scope, truncated: bool) -> ApiResult<Completeness> {
    let wms = state.pg.watermarks(PROCESSOR, TOPIC).await?;
    let dead = state.pg.count_unresolved_processing_errors(scope.tenant_id.as_str()).await?;
    Ok(completeness_from(&wms, dead, truncated))
}

fn completeness_from(wms: &[WatermarkRow], dead: i64, truncated: bool) -> Completeness {
    let mut caught_up = !wms.is_empty();
    let mut watermark: Option<i64> = None;
    for w in wms {
        let at = if w.next_offset >= w.high_watermark {
            (w.updated_at.unix_timestamp_nanos()).clamp(0, i64::MAX as i128) as i64
        } else {
            caught_up = false;
            w.last_received_time_ns
        };
        watermark = Some(watermark.map_or(at, |m| m.min(at)));
    }
    let mut notes = vec!["Only instrumented components are visible.".to_owned()];
    if !caught_up {
        notes.push("Processing is behind ingestion; recent data may be missing.".into());
    }
    if dead > 0 {
        notes.push(format!("{dead} record(s) failed processing and are not included."));
    }
    Completeness {
        watermark_ns: watermark.map(ns),
        processing_caught_up: caught_up,
        unresolved_processing_errors: dead,
        truncated,
        sampling: "unknown".into(),
        notes,
    }
}

fn nonempty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

// ---------------------------------------------------------------- services

#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceDto {
    pub service_id: String,
    pub name: String,
    pub requests: u64,
    pub errors: u64,
    pub p95_ms: Option<f64>,
    pub last_seen_ns: Option<String>,
    pub instances: u64,
    pub version: Option<String>,
    pub logs: u64,
    pub error_logs: u64,
    /// Signals seen for this service in the range (`traces`, `logs`).
    pub signals: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServicesResponse {
    pub services: Vec<ServiceDto>,
    pub from_ns: String,
    pub to_ns: String,
    pub is_demo: bool,
    pub completeness: Completeness,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/services", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), RangeParams),
    responses((status = 200, body = ServicesResponse), (status = 404, body = crate::error::ErrorBody)))]
pub async fn services(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<RangeParams>,
) -> ApiResult<Json<ServicesResponse>> {
    let (scope, is_demo) = authorize(&state, &user, &project_id, &environment_id, Role::Viewer).await?;
    let r = q.resolve()?;
    let (spans, logs) = tokio::try_join!(
        state.ch.service_summaries(&scope, r.from, r.to),
        state.ch.log_services(&scope, r.from, r.to)
    )?;
    Ok(Json(ServicesResponse {
        services: merge_services(spans, logs),
        from_ns: ns(r.from),
        to_ns: ns(r.to),
        is_demo,
        completeness: completeness(&state, &scope, false).await?,
    }))
}

/// One entry per service seen in spans and/or logs, sorted by service ID.
fn merge_services(spans: Vec<ServiceSummaryRow>, logs: Vec<LogServiceRow>) -> Vec<ServiceDto> {
    let mut out: BTreeMap<String, ServiceDto> = BTreeMap::new();
    for s in spans {
        out.insert(
            s.service_id.clone(),
            ServiceDto {
                service_id: s.service_id,
                name: s.service_name,
                requests: s.requests,
                errors: s.errors,
                p95_ms: (s.requests > 0 && s.p95_ns.is_finite()).then(|| s.p95_ns / 1e6),
                last_seen_ns: Some(ns(s.last_seen)),
                instances: s.instances,
                version: nonempty(s.version),
                logs: 0,
                error_logs: 0,
                signals: vec!["traces".into()],
            },
        );
    }
    for l in logs {
        let e = out.entry(l.service_id.clone()).or_insert_with(|| ServiceDto {
            service_id: l.service_id.clone(),
            name: l.service_name.clone(),
            requests: 0,
            errors: 0,
            p95_ms: None,
            last_seen_ns: None,
            instances: 0,
            version: None,
            logs: 0,
            error_logs: 0,
            signals: vec![],
        });
        e.logs = l.logs;
        e.error_logs = l.error_logs;
        e.signals.push("logs".into());
        let last = e.last_seen_ns.as_ref().and_then(|v| v.parse::<i64>().ok()).unwrap_or(0).max(l.last_seen);
        e.last_seen_ns = Some(ns(last));
    }
    out.into_values().collect()
}

// ---------------------------------------------------------------- topology

#[derive(Debug, Serialize, ToSchema)]
pub struct TopologyNodeDto {
    pub id: String,
    pub name: String,
    pub kind: EntityKindDto,
    pub provenance: ProvenanceDto,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TopologyEdgeDto {
    pub source: String,
    pub target: String,
    pub relation: RelationKindDto,
    pub provenance: ProvenanceDto,
    /// Evidence for inferred relationships. No numeric confidence is invented.
    pub reasons: Vec<String>,
    pub calls: u64,
    pub errors: u64,
    pub last_seen_ns: String,
}

/// Superset of the engine's `TopologySnapshot`; the Worker passes it to WASM unchanged.
#[derive(Debug, Serialize, ToSchema)]
pub struct TopologyResponse {
    pub nodes: Vec<TopologyNodeDto>,
    pub edges: Vec<TopologyEdgeDto>,
    pub from_ns: String,
    pub to_ns: String,
    pub completeness: Completeness,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/topology", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), RangeParams),
    responses((status = 200, body = TopologyResponse), (status = 404, body = crate::error::ErrorBody)))]
pub async fn topology(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<RangeParams>,
) -> ApiResult<Json<TopologyResponse>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Viewer).await?;
    let r = q.resolve()?;
    let (services, logs, calls, resources, clients) = tokio::try_join!(
        state.ch.service_summaries(&scope, r.from, r.to),
        state.ch.log_services(&scope, r.from, r.to),
        state.ch.call_edges(&scope, r.from, r.to),
        state.ch.resource_edges(&scope, r.from, r.to),
        state.ch.external_client_edges(&scope, r.from, r.to),
    )?;
    let (nodes, edges) = build_topology(&services, &logs, calls, resources, clients, &external_clients_id(&scope));
    Ok(Json(TopologyResponse {
        nodes,
        edges,
        from_ns: ns(r.from),
        to_ns: ns(r.to),
        completeness: completeness(&state, &scope, false).await?,
    }))
}

/// One node per environment for everything that calls it without being instrumented
/// (browser, frontend, curl, partner systems). A fixed set keeps untrusted telemetry from
/// creating unbounded nodes.
pub(crate) fn external_clients_id(scope: &Scope) -> String {
    EntityId::derive_resource(scope, "client", "external", "clients").to_string()
}

const EXTERNAL_CLIENTS_NAME: &str = "External clients";

fn build_topology(
    services: &[ServiceSummaryRow],
    logs: &[LogServiceRow],
    calls: Vec<CallEdgeRow>,
    resources: Vec<ResourceEdgeRow>,
    clients: Vec<ExternalClientEdgeRow>,
    clients_id: &str,
) -> (Vec<TopologyNodeDto>, Vec<TopologyEdgeDto>) {
    let mut nodes: BTreeMap<String, TopologyNodeDto> = BTreeMap::new();
    for (id, name) in services
        .iter()
        .map(|s| (&s.service_id, &s.service_name))
        .chain(logs.iter().map(|l| (&l.service_id, &l.service_name)))
    {
        nodes.entry(id.clone()).or_insert_with(|| TopologyNodeDto {
            id: id.clone(),
            name: name.clone(),
            kind: EntityKindDto::Service,
            provenance: ProvenanceDto::Observed,
        });
    }
    let mut edges = Vec::new();
    for c in calls {
        edges.push(TopologyEdgeDto {
            source: c.source_service_id,
            target: c.target_service_id,
            relation: RelationKindDto::Calls,
            provenance: ProvenanceDto::Observed,
            reasons: vec![],
            calls: c.calls,
            errors: c.errors,
            last_seen_ns: ns(c.last_seen),
        });
    }
    for rsc in resources {
        let kind = match rsc.target_kind.as_str() {
            "database" => EntityKindDto::Database,
            "queue" => EntityKindDto::Queue,
            _ => EntityKindDto::ExternalDependency,
        };
        let reason = if kind == EntityKindDto::ExternalDependency {
            format!(
                "HTTP client spans call '{}' and no service in this environment answered: an external service outside the project",
                rsc.target_name
            )
        } else {
            format!(
                "client spans carry {} semantic-convention attributes for '{}'; the resource itself is not instrumented",
                rsc.target_kind, rsc.target_name
            )
        };
        nodes.entry(rsc.target_entity_id.clone()).or_insert_with(|| TopologyNodeDto {
            id: rsc.target_entity_id.clone(),
            name: rsc.target_name.clone(),
            kind,
            provenance: ProvenanceDto::Inferred,
        });
        edges.push(TopologyEdgeDto {
            source: rsc.service_id,
            target: rsc.target_entity_id,
            relation: RelationKindDto::DependsOn,
            provenance: ProvenanceDto::Inferred,
            reasons: vec![reason],
            calls: rsc.calls,
            errors: rsc.errors,
            last_seen_ns: ns(rsc.last_seen),
        });
    }
    for c in clients {
        nodes.entry(clients_id.to_owned()).or_insert_with(|| TopologyNodeDto {
            id: clients_id.to_owned(),
            name: EXTERNAL_CLIENTS_NAME.to_owned(),
            kind: EntityKindDto::ExternalClient,
            provenance: ProvenanceDto::Inferred,
        });
        edges.push(TopologyEdgeDto {
            source: clients_id.to_owned(),
            target: c.service_id,
            relation: RelationKindDto::Calls,
            provenance: ProvenanceDto::Inferred,
            reasons: vec![
                "server spans without an instrumented caller: requests from outside the project (browser, frontend, external system)".into(),
            ],
            calls: c.calls,
            errors: c.errors,
            last_seen_ns: ns(c.last_seen),
        });
    }
    (nodes.into_values().collect(), edges)
}

// ---------------------------------------------------------------- traces

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TraceListParams {
    pub from: Option<String>,
    pub to: Option<String>,
    pub service_id: Option<String>,
    #[serde(default)]
    pub only_errors: bool,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceSummaryDto {
    pub trace_id: String,
    pub start_ns: String,
    pub end_ns: String,
    pub duration_ms: f64,
    pub span_count: u64,
    pub error_count: u64,
    pub root_name: String,
    pub root_service: String,
    pub services: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceListResponse {
    pub traces: Vec<TraceSummaryDto>,
    pub next_cursor: Option<String>,
    pub completeness: Completeness,
}

#[derive(Debug, Serialize, Deserialize)]
struct Cursor {
    t: String,
    id: String,
}

fn encode_cursor(t: i64, id: &str) -> String {
    let json = serde_json::to_vec(&Cursor { t: ns(t), id: id.to_owned() }).unwrap_or_default();
    URL_SAFE_NO_PAD.encode(json)
}

fn decode_cursor(c: &Option<String>) -> ApiResult<Option<(i64, String)>> {
    let Some(c) = c else { return Ok(None) };
    let bad = || ApiError::BadRequest("invalid cursor".into());
    let bytes = URL_SAFE_NO_PAD.decode(c).map_err(|_| bad())?;
    let cur: Cursor = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    let t = cur.t.parse::<i64>().map_err(|_| bad())?;
    if cur.id.len() > 256 {
        return Err(bad());
    }
    Ok(Some((t, cur.id)))
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/traces", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), TraceListParams),
    responses((status = 200, body = TraceListResponse), (status = 404, body = crate::error::ErrorBody)))]
pub async fn traces(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<TraceListParams>,
) -> ApiResult<Json<TraceListResponse>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Viewer).await?;
    let (query, limit) = q.to_query()?;
    let rows = state.ch.traces(&scope, &query).await?;
    let (traces, next_cursor, has_more) = trace_page(rows, limit);
    Ok(Json(TraceListResponse { traces, next_cursor, completeness: completeness(&state, &scope, has_more).await? }))
}

impl TraceListParams {
    /// Validated storage query (one extra row to detect more pages) and the page size.
    fn to_query(&self) -> ApiResult<(TraceQuery, u32)> {
        let r = RangeParams { from: self.from.clone(), to: self.to.clone() }.resolve()?;
        let limit = limit(self.limit, 50, 500);
        let query = TraceQuery {
            from_ns: r.from,
            to_ns: r.to,
            service_id: self.service_id.clone(),
            only_errors: self.only_errors,
            before: decode_cursor(&self.cursor)?,
            limit: limit + 1,
        };
        Ok((query, limit))
    }
}

fn trace_page(rows: Vec<TraceSummaryRow>, limit: u32) -> (Vec<TraceSummaryDto>, Option<String>, bool) {
    let has_more = rows.len() > limit as usize;
    let traces: Vec<TraceSummaryDto> = rows
        .into_iter()
        .take(limit as usize)
        .map(|t| TraceSummaryDto {
            duration_ms: (t.trace_end - t.trace_start) as f64 / 1e6,
            start_ns: ns(t.trace_start),
            end_ns: ns(t.trace_end),
            trace_id: t.trace_id,
            span_count: t.span_count,
            error_count: t.error_count,
            root_name: t.root_name,
            root_service: t.root_service,
            services: t.services,
        })
        .collect();
    let next_cursor = if has_more {
        traces.last().and_then(|t| t.start_ns.parse::<i64>().ok().map(|s| encode_cursor(s, &t.trace_id)))
    } else {
        None
    };
    (traces, next_cursor, has_more)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SpanDto {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub service_id: String,
    pub service_name: String,
    pub service_instance_id: Option<String>,
    pub service_version: Option<String>,
    pub name: String,
    pub kind: u8,
    pub start_ns: String,
    pub end_ns: String,
    pub duration_ms: f64,
    /// OTLP status: 0 unset, 1 ok, 2 error.
    pub status_code: u8,
    pub status_message: String,
    pub target_entity_id: Option<String>,
    pub target_name: Option<String>,
    /// Attributes after server-side redaction.
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogDto {
    pub id: String,
    pub time_ns: String,
    pub observed_time_ns: String,
    pub service_id: String,
    pub service_name: String,
    pub severity_number: u8,
    pub severity_text: String,
    /// Untrusted text. Render as plain text only.
    pub body: String,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventDto {
    pub id: String,
    pub event_type: String,
    pub service_id: String,
    pub service_name: String,
    pub entity_id: Option<String>,
    pub time_ns: String,
    pub observed_time_ns: String,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub source_id: Option<String>,
    pub source_epoch: Option<String>,
    /// Decimal string (u64 does not fit a JS number).
    pub source_sequence: Option<String>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TraceDetailResponse {
    pub trace_id: String,
    pub spans: Vec<SpanDto>,
    /// Parent span IDs referenced by spans in this trace but not recorded
    /// (not instrumented, sampled out, dropped or still in flight).
    pub missing_parent_span_ids: Vec<String>,
    pub logs: Vec<LogDto>,
    pub events: Vec<EventDto>,
    pub truncated: bool,
    pub completeness: Completeness,
}

fn log_dto(l: LogRow) -> LogDto {
    LogDto {
        id: l.log_id,
        time_ns: ns(l.time),
        observed_time_ns: ns(l.observed_time),
        service_id: l.service_id,
        service_name: l.service_name,
        severity_number: l.severity_number,
        severity_text: l.severity_text,
        body: l.body,
        trace_id: nonempty(l.trace_id),
        span_id: nonempty(l.span_id),
        attributes: l.attributes.into_iter().collect(),
    }
}

fn event_dto(e: controlhub_storage::ch::EventRow) -> EventDto {
    EventDto {
        id: e.event_id,
        event_type: e.event_type,
        service_id: e.service_id,
        service_name: e.service_name,
        entity_id: nonempty(e.entity_id),
        time_ns: ns(e.event_time),
        observed_time_ns: ns(e.observed_time),
        trace_id: nonempty(e.trace_id),
        span_id: nonempty(e.span_id),
        source_id: nonempty(e.source_id),
        source_epoch: nonempty(e.source_epoch),
        source_sequence: e.source_sequence.map(|s| s.to_string()),
        attributes: e.attributes.into_iter().collect(),
    }
}

/// Parent span IDs referenced in the trace but not recorded, sorted and unique.
fn missing_parents(spans: &[SpanRow]) -> Vec<String> {
    let ids: HashSet<&str> = spans.iter().map(|s| s.span_id.as_str()).collect();
    let mut missing: Vec<String> = spans
        .iter()
        .filter(|s| !s.parent_span_id.is_empty() && !ids.contains(s.parent_span_id.as_str()))
        .map(|s| s.parent_span_id.clone())
        .collect();
    missing.sort();
    missing.dedup();
    missing
}

fn span_dto(s: SpanRow) -> SpanDto {
    SpanDto {
        duration_ms: s.duration_ns as f64 / 1e6,
        trace_id: s.trace_id,
        span_id: s.span_id,
        parent_span_id: nonempty(s.parent_span_id),
        service_id: s.service_id,
        service_name: s.service_name,
        service_instance_id: nonempty(s.service_instance_id),
        service_version: nonempty(s.service_version),
        name: s.name,
        kind: s.kind,
        start_ns: ns(s.start_time),
        end_ns: ns(s.end_time),
        status_code: s.status_code,
        status_message: s.status_message,
        target_entity_id: nonempty(s.target_entity_id),
        target_name: nonempty(s.target_name),
        attributes: s.attributes.into_iter().collect(),
    }
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/traces/{trace_id}", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), ("trace_id" = String, Path)),
    responses((status = 200, body = TraceDetailResponse), (status = 404, body = crate::error::ErrorBody)))]
pub async fn trace_detail(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id, trace_id)): Path<(String, String, String)>,
) -> ApiResult<Json<TraceDetailResponse>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Viewer).await?;
    let trace_id = TraceId::parse(trace_id).map_err(|_| ApiError::BadRequest("invalid trace id".into()))?;
    const SPAN_LIMIT: u32 = 5_000;
    let spans = state.ch.trace_spans(&scope, trace_id.as_str(), SPAN_LIMIT + 1).await?;
    if spans.is_empty() {
        return Err(ApiError::NotFound);
    }
    let truncated = spans.len() > SPAN_LIMIT as usize;
    let min = spans.iter().map(|s| s.start_time).min().unwrap_or(0);
    let max = spans.iter().map(|s| s.end_time).max().unwrap_or(0);
    let (from, to) = (min.saturating_sub(NS_PER_MIN), max.saturating_add(NS_PER_MIN));
    let log_query = LogQuery {
        from_ns: from,
        to_ns: to,
        trace_id: Some(trace_id.to_string()),
        limit: 1_000,
        ..LogQuery::default()
    };
    let (logs, events) = tokio::try_join!(
        state.ch.logs(&scope, &log_query),
        state.ch.events(&scope, from, to, Some(trace_id.as_str()), 1_000),
    )?;
    let missing = missing_parents(&spans);
    let span_dtos = spans.into_iter().take(SPAN_LIMIT as usize).map(span_dto).collect();
    let mut logs: Vec<LogDto> = logs.into_iter().map(log_dto).collect();
    logs.reverse();
    let mut events: Vec<EventDto> = events.into_iter().map(event_dto).collect();
    events.reverse();
    Ok(Json(TraceDetailResponse {
        trace_id: trace_id.to_string(),
        spans: span_dtos,
        missing_parent_span_ids: missing,
        logs,
        events,
        truncated,
        completeness: completeness(&state, &scope, truncated).await?,
    }))
}

// ---------------------------------------------------------------- logs & events

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LogParams {
    pub from: Option<String>,
    pub to: Option<String>,
    pub service_id: Option<String>,
    /// Minimum OTLP severity number (e.g. 17 = ERROR).
    pub min_severity: Option<u8>,
    pub trace_id: Option<String>,
    /// Case-insensitive substring match on the log body.
    pub q: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogListResponse {
    /// Newest first.
    pub logs: Vec<LogDto>,
    pub next_cursor: Option<String>,
    pub completeness: Completeness,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/logs", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), LogParams),
    responses((status = 200, body = LogListResponse), (status = 404, body = crate::error::ErrorBody)))]
pub async fn logs(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<LogParams>,
) -> ApiResult<Json<LogListResponse>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Viewer).await?;
    let (query, limit) = q.to_query()?;
    let rows = state.ch.logs(&scope, &query).await?;
    let (logs, next_cursor, has_more) = log_page(rows, limit);
    Ok(Json(LogListResponse { logs, next_cursor, completeness: completeness(&state, &scope, has_more).await? }))
}

impl LogParams {
    /// Validated storage query (one extra row to detect more pages) and the page size.
    fn to_query(&self) -> ApiResult<(LogQuery, u32)> {
        let r = RangeParams { from: self.from.clone(), to: self.to.clone() }.resolve()?;
        let limit = limit(self.limit, 200, 1_000);
        if let Some(t) = &self.trace_id {
            TraceId::parse(t.clone()).map_err(|_| ApiError::BadRequest("invalid trace id".into()))?;
        }
        let text = self.q.clone().filter(|s| !s.trim().is_empty());
        if text.as_ref().is_some_and(|s| s.len() > 256) {
            return Err(ApiError::BadRequest("q too long".into()));
        }
        let query = LogQuery {
            from_ns: r.from,
            to_ns: r.to,
            service_id: self.service_id.clone(),
            min_severity: self.min_severity,
            trace_id: self.trace_id.clone(),
            text,
            before: decode_cursor(&self.cursor)?,
            limit: limit + 1,
        };
        Ok((query, limit))
    }
}

fn log_page(rows: Vec<LogRow>, limit: u32) -> (Vec<LogDto>, Option<String>, bool) {
    let has_more = rows.len() > limit as usize;
    let logs: Vec<LogDto> = rows.into_iter().take(limit as usize).map(log_dto).collect();
    let next_cursor = if has_more {
        logs.last().and_then(|l| l.time_ns.parse::<i64>().ok().map(|t| encode_cursor(t, &l.id)))
    } else {
        None
    };
    (logs, next_cursor, has_more)
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventParams {
    pub from: Option<String>,
    pub to: Option<String>,
    pub trace_id: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventListResponse {
    pub events: Vec<EventDto>,
    pub completeness: Completeness,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/events", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), EventParams),
    responses((status = 200, body = EventListResponse), (status = 404, body = crate::error::ErrorBody)))]
pub async fn events(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<EventParams>,
) -> ApiResult<Json<EventListResponse>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Viewer).await?;
    let r = RangeParams { from: q.from.clone(), to: q.to.clone() }.resolve()?;
    let limit = limit(q.limit, 200, 1_000);
    let rows = state.ch.events(&scope, r.from, r.to, q.trace_id.as_deref(), limit + 1).await?;
    let truncated = rows.len() > limit as usize;
    Ok(Json(EventListResponse {
        events: rows.into_iter().take(limit as usize).map(event_dto).collect(),
        completeness: completeness(&state, &scope, truncated).await?,
    }))
}

// ---------------------------------------------------------------- timeline

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimelineParams {
    pub from: Option<String>,
    pub to: Option<String>,
    /// Max items per signal (default 20000, max 100000).
    pub limit: Option<u32>,
}

/// Directly consumable by the engine's `applyBatch` (it reads `items`).
#[derive(Debug, Serialize, ToSchema)]
pub struct TimelineResponse {
    pub items: Vec<TimelineItem>,
    pub from_ns: String,
    pub to_ns: String,
    /// Per-signal truncation: when true, the range holds more items than returned.
    pub spans_truncated: bool,
    pub logs_truncated: bool,
    pub events_truncated: bool,
    pub completeness: Completeness,
}

/// `clients_id`: the environment's external clients node (see [`external_clients_id`]), set as
/// the source of root server spans.
pub(crate) fn span_item(s: TimelineSpanRow, clients_id: &str) -> TimelineItem {
    let from_outside = s.kind == 2 && s.parent_span_id.is_empty();
    TimelineItem::Span(SpanItem {
        trace_id: s.trace_id,
        span_id: s.span_id,
        parent_span_id: nonempty(s.parent_span_id),
        service_id: s.service_id,
        service_name: s.service_name,
        target_entity_id: nonempty(s.target_entity_id),
        source_entity_id: from_outside.then(|| clients_id.to_owned()),
        name: s.name,
        kind: s.kind,
        start_ns: s.start_time,
        end_ns: s.end_time,
        is_error: s.status_code == 2,
    })
}

pub(crate) fn log_item(l: TimelineLogRow) -> TimelineItem {
    TimelineItem::Log(LogItem {
        id: l.log_id,
        service_id: l.service_id,
        service_name: l.service_name,
        time_ns: l.time,
        severity: l.severity_number,
        trace_id: nonempty(l.trace_id),
    })
}

pub(crate) fn event_item(e: EventRow) -> TimelineItem {
    TimelineItem::Event(EventItem {
        id: e.event_id,
        event_type: e.event_type,
        service_id: nonempty(e.service_id),
        service_name: nonempty(e.service_name),
        time_ns: e.event_time,
        trace_id: nonempty(e.trace_id),
    })
}

/// At most `limit` items per signal, with a per-signal truncation flag.
fn timeline_items(
    spans: Vec<TimelineSpanRow>,
    logs: Vec<TimelineLogRow>,
    events: Vec<EventRow>,
    limit: u32,
    clients_id: &str,
) -> (Vec<TimelineItem>, bool, bool, bool) {
    let (st, lt, et) = (spans.len() > limit as usize, logs.len() > limit as usize, events.len() > limit as usize);
    let mut items = Vec::with_capacity(spans.len() + logs.len() + events.len());
    items.extend(spans.into_iter().take(limit as usize).map(|s| span_item(s, clients_id)));
    items.extend(logs.into_iter().take(limit as usize).map(log_item));
    items.extend(events.into_iter().take(limit as usize).map(event_item));
    (items, st, lt, et)
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/timeline", tag = "timeline",
    params(("project_id" = String, Path), ("environment_id" = String, Path), TimelineParams),
    responses((status = 200, body = TimelineResponse), (status = 404, body = crate::error::ErrorBody)))]
pub async fn timeline(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<TimelineParams>,
) -> ApiResult<Json<TimelineResponse>> {
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Viewer).await?;
    let r = RangeParams { from: q.from.clone(), to: q.to.clone() }.resolve()?;
    let limit = limit(q.limit, 20_000, 100_000);
    let (spans, logs, events) = tokio::try_join!(
        state.ch.timeline_spans(&scope, r.from, r.to, limit + 1),
        state.ch.timeline_logs(&scope, r.from, r.to, limit + 1),
        state.ch.events(&scope, r.from, r.to, None, limit + 1),
    )?;
    let (items, st, lt, et) = timeline_items(spans, logs, events, limit, &external_clients_id(&scope));
    Ok(Json(TimelineResponse {
        items,
        from_ns: ns(r.from),
        to_ns: ns(r.to),
        spans_truncated: st,
        logs_truncated: lt,
        events_truncated: et,
        completeness: completeness(&state, &scope, st || lt || et).await?,
    }))
}

// ---------------------------------------------------------------- coverage

#[derive(Debug, Serialize, ToSchema)]
pub struct CoverageResponse {
    pub services: Vec<ServiceDto>,
    /// Capabilities that are explicitly NOT available, so gaps are visible.
    pub unsupported: Vec<String>,
    pub completeness: Completeness,
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/coverage", tag = "telemetry",
    params(("project_id" = String, Path), ("environment_id" = String, Path), RangeParams),
    responses((status = 200, body = CoverageResponse), (status = 404, body = crate::error::ErrorBody)))]
pub async fn coverage(
    state: State<AppState>,
    user: CurrentUser,
    path: Path<(String, String)>,
    query: Query<RangeParams>,
) -> ApiResult<Json<CoverageResponse>> {
    let Json(svc) = services(state, user, path, query).await?;
    Ok(Json(CoverageResponse {
        services: svc.services,
        unsupported: vec![
            "OTLP metrics ingestion (not implemented yet)".into(),
            "OTLP/HTTP JSON encoding (protobuf only)".into(),
            "SDK sampling configuration is not reported; sampled-out requests are invisible".into(),
            "Uninstrumented services/resources appear only as inferred dependencies, if at all".into(),
        ],
        completeness: svc.completeness,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    const T0: i64 = 1_790_000_000_000_000_000;

    fn range(from: Option<i64>, to: Option<i64>) -> ApiResult<Range> {
        RangeParams { from: from.map(|v| v.to_string()), to: to.map(|v| v.to_string()) }.resolve()
    }

    fn bad_request(r: ApiResult<impl std::fmt::Debug>) -> String {
        match r {
            Err(ApiError::BadRequest(m)) => m,
            other => panic!("expected 400, got {other:?}"),
        }
    }

    #[test]
    fn range_defaults_to_the_last_15_minutes() {
        let r = range(None, Some(T0)).unwrap();
        assert_eq!((r.from, r.to), (T0 - 15 * NS_PER_MIN, T0));
        let before = now_ns();
        let r = range(None, None).unwrap();
        assert!(r.to >= before && r.to - r.from == 15 * NS_PER_MIN);
    }

    #[test]
    fn range_must_be_ordered_and_at_most_7_days() {
        assert!(bad_request(range(Some(T0), Some(T0))).contains("before"));
        assert!(bad_request(range(Some(T0 + 1), Some(T0))).contains("before"));
        assert!(range(Some(T0 - MAX_RANGE_NS), Some(T0)).is_ok(), "exactly 7 days");
        assert!(bad_request(range(Some(T0 - MAX_RANGE_NS - 1), Some(T0))).contains("7 days"));
        // A default `from` is 15 minutes before an explicit far-past `to`, which is fine.
        assert!(range(None, Some(1)).is_ok());
    }

    #[test]
    fn extreme_ranges_do_not_wrap_around() {
        // Regression: `to - from` used to overflow (panic in debug, pass as -1 in release).
        assert!(bad_request(range(Some(i64::MIN), Some(i64::MAX))).contains("7 days"));
        assert!(bad_request(range(Some(-1), Some(i64::MAX))).contains("7 days"));
        assert!(bad_request(range(Some(i64::MIN), Some(0))).contains("7 days"));
        let r = range(None, Some(i64::MIN + 1)).unwrap();
        assert_eq!(r.from, i64::MIN, "default `from` saturates instead of overflowing");
    }

    #[test]
    fn timestamps_must_be_plain_decimal_integers() {
        for v in ["", "abc", "1e18", "1.5", "0x10", " 1", "1 ", "+-1", "9223372036854775808", "1_000"] {
            let m = bad_request(RangeParams { from: Some(v.into()), to: Some(T0.to_string()) }.resolve());
            assert!(m.contains("from"), "{v:?}: {m}");
            let m = bad_request(RangeParams { from: None, to: Some(v.into()) }.resolve());
            assert!(m.contains("to"), "{v:?}: {m}");
        }
        assert_eq!(parse_ns(&Some("-5".into()), "x").unwrap(), Some(-5));
        assert_eq!(parse_ns(&None, "x").unwrap(), None);
    }

    proptest! {
        /// Any pair of timestamps either resolves to a valid, bounded range or is a 400; never a panic.
        #[test]
        fn resolve_never_panics_and_bounds_hold(from in any::<i64>(), to in any::<i64>()) {
            match range(Some(from), Some(to)) {
                Ok(r) => prop_assert!(r.from < r.to && r.to - r.from <= MAX_RANGE_NS),
                Err(ApiError::BadRequest(_)) => prop_assert!(from >= to || (to as i128 - from as i128) > MAX_RANGE_NS as i128),
                Err(e) => prop_assert!(false, "unexpected {:?}", e),
            }
        }

        #[test]
        fn cursors_round_trip(t in any::<i64>(), id in "[ -~]{0,256}") {
            let c = encode_cursor(t, &id);
            prop_assert!(c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "URL-safe: {}", c);
            prop_assert_eq!(decode_cursor(&Some(c)).unwrap(), Some((t, id)));
        }

        /// Arbitrary client cursors decode or fail with 400, never panic.
        #[test]
        fn arbitrary_cursors_are_rejected_cleanly(c in ".{0,400}") {
            match decode_cursor(&Some(c)) {
                Ok(Some((_, id))) => prop_assert!(id.len() <= 256),
                Ok(None) => prop_assert!(false, "Some input never decodes to None"),
                Err(ApiError::BadRequest(m)) => prop_assert_eq!(m, "invalid cursor"),
                Err(e) => prop_assert!(false, "unexpected {:?}", e),
            }
        }
    }

    #[test]
    fn invalid_cursors_are_400() {
        let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(serde_json::to_vec(&v).unwrap());
        for c in [
            "!!!".to_owned(),
            "eyJ0IjoiMSIsImlkIjoiYSJ9==".to_owned(), // padded
            URL_SAFE_NO_PAD.encode(b"not json"),
            enc(json!({ "t": 1, "id": "a" })), // t must be a decimal string
            enc(json!({ "t": "abc", "id": "a" })),
            enc(json!({ "t": "1" })),
            enc(json!({ "t": "1", "id": "x".repeat(257) })),
            enc(json!([1, 2])),
        ] {
            assert_eq!(bad_request(decode_cursor(&Some(c.clone()))), "invalid cursor", "{c}");
        }
        assert_eq!(decode_cursor(&None).unwrap(), None);
        assert!(decode_cursor(&Some(enc(json!({ "t": "1", "id": "x".repeat(256) })))).is_ok());
    }

    #[test]
    fn limits_are_clamped() {
        assert_eq!(limit(None, 50, 500), 50);
        assert_eq!(limit(Some(0), 50, 500), 1);
        assert_eq!(limit(Some(501), 50, 500), 500);
        assert_eq!(limit(Some(u32::MAX), 20_000, 100_000), 100_000);
    }

    fn trace_params() -> TraceListParams {
        TraceListParams {
            from: Some((T0 - 1000).to_string()),
            to: Some(T0.to_string()),
            service_id: None,
            only_errors: false,
            limit: None,
            cursor: None,
        }
    }

    #[test]
    fn trace_params_build_a_bounded_query() {
        let (q, l) = trace_params().to_query().unwrap();
        assert_eq!((l, q.limit, q.from_ns, q.to_ns), (50, 51, T0 - 1000, T0));
        let (q, l) = TraceListParams { limit: Some(10_000), ..trace_params() }.to_query().unwrap();
        assert_eq!((l, q.limit), (500, 501));
        let evil = "a' OR '1'='1".to_owned();
        let p = TraceListParams {
            service_id: Some(evil.clone()),
            only_errors: true,
            cursor: Some(encode_cursor(T0 - 10, "tid")),
            ..trace_params()
        };
        let (q, _) = p.to_query().unwrap();
        assert_eq!(q.service_id, Some(evil), "passed through as a bound parameter, unmodified");
        assert!(q.only_errors);
        assert_eq!(q.before, Some((T0 - 10, "tid".into())));
        assert!(TraceListParams { cursor: Some("zz".into()), ..trace_params() }.to_query().is_err());
        assert!(TraceListParams { from: Some("x".into()), ..trace_params() }.to_query().is_err());
    }

    fn log_params() -> LogParams {
        LogParams {
            from: Some((T0 - 1000).to_string()),
            to: Some(T0.to_string()),
            service_id: None,
            min_severity: None,
            trace_id: None,
            q: None,
            limit: None,
            cursor: None,
        }
    }

    #[test]
    fn log_params_are_validated() {
        let (q, l) = log_params().to_query().unwrap();
        assert_eq!((l, q.limit), (200, 201));
        assert_eq!(LogParams { limit: Some(0), ..log_params() }.to_query().unwrap().1, 1);
        assert_eq!(LogParams { limit: Some(5_000), ..log_params() }.to_query().unwrap().1, 1_000);

        let tid = "0123456789abcdef0123456789abcdef".to_owned();
        let (q, _) = LogParams { trace_id: Some(tid.clone()), ..log_params() }.to_query().unwrap();
        assert_eq!(q.trace_id, Some(tid));
        for bad in ["", "xyz", "0123456789ABCDEF0123456789ABCDEF", "0123456789abcdef0123456789abcde", "' OR 1=1 --"] {
            let m = bad_request(LogParams { trace_id: Some(bad.into()), ..log_params() }.to_query());
            assert_eq!(m, "invalid trace id", "{bad:?}");
        }

        assert_eq!(LogParams { q: Some("   ".into()), ..log_params() }.to_query().unwrap().0.text, None);
        let (q, _) = LogParams { q: Some("%' OR 1=1 --".into()), ..log_params() }.to_query().unwrap();
        assert_eq!(q.text.as_deref(), Some("%' OR 1=1 --"), "search text is data, bound as a parameter");
        assert!(LogParams { q: Some("x".repeat(256)), ..log_params() }.to_query().is_ok());
        assert_eq!(bad_request(LogParams { q: Some("x".repeat(257)), ..log_params() }.to_query()), "q too long");
        assert_eq!(
            bad_request(LogParams { q: Some("é".repeat(129)), ..log_params() }.to_query()),
            "q too long",
            "bytes"
        );
        assert!(LogParams { cursor: Some("?".into()), ..log_params() }.to_query().is_err());
    }

    fn trace_row(id: &str, start: i64) -> TraceSummaryRow {
        TraceSummaryRow {
            trace_id: id.into(),
            trace_start: start,
            trace_end: start + 2_500_000,
            span_count: 3,
            error_count: 1,
            root_name: "GET /".into(),
            root_service: "gateway".into(),
            services: vec!["gateway".into()],
        }
    }

    #[test]
    fn trace_pages_report_more_and_point_the_cursor_at_the_last_item() {
        let rows: Vec<_> = (0..4).map(|i| trace_row(&format!("t{i}"), T0 - i)).collect();
        let (page, cursor, more) = trace_page(rows.clone(), 3);
        assert!(more);
        assert_eq!(page.len(), 3);
        assert_eq!(page[0].duration_ms, 2.5);
        assert_eq!(page[0].start_ns, T0.to_string());
        assert_eq!(decode_cursor(&cursor).unwrap(), Some((T0 - 2, "t2".into())));

        let (page, cursor, more) = trace_page(rows[..3].to_vec(), 3);
        assert!(!more && cursor.is_none() && page.len() == 3);
        let (page, cursor, more) = trace_page(vec![], 3);
        assert!(!more && cursor.is_none() && page.is_empty());
    }

    fn log_row(id: &str, time: i64) -> LogRow {
        LogRow {
            tenant_id: "t".into(),
            project_id: "p".into(),
            environment_id: "e".into(),
            service_id: "svc_a".into(),
            service_name: "a".into(),
            service_instance_id: String::new(),
            log_id: id.into(),
            time,
            observed_time: time + 1,
            severity_number: 17,
            severity_text: "ERROR".into(),
            body: "<script>alert(1)</script>".into(),
            trace_id: String::new(),
            span_id: "0123456789abcdef".into(),
            attributes: vec![("b".into(), "2".into()), ("a".into(), "1".into())],
            resource_attributes: vec![],
            batch_id: "b".into(),
            ingested_at: time,
        }
    }

    #[test]
    fn log_pages_and_dtos() {
        let rows: Vec<_> = (0..3).map(|i| log_row(&format!("l{i}"), T0 - i)).collect();
        let (page, cursor, more) = log_page(rows.clone(), 2);
        assert!(more && page.len() == 2);
        assert_eq!(decode_cursor(&cursor).unwrap(), Some((T0 - 1, "l1".into())));
        let (_, cursor, more) = log_page(rows, 3);
        assert!(!more && cursor.is_none());

        let d = log_dto(log_row("l", T0));
        assert_eq!(d.body, "<script>alert(1)</script>", "untrusted text is passed as data, never transformed");
        assert_eq!(d.trace_id, None, "empty strings become null");
        assert_eq!(d.span_id.as_deref(), Some("0123456789abcdef"));
        assert_eq!(d.observed_time_ns, (T0 + 1).to_string());
        assert_eq!(d.attributes.keys().collect::<Vec<_>>(), ["a", "b"], "attributes are sorted");
    }

    fn event_row(id: &str) -> EventRow {
        EventRow {
            tenant_id: "t".into(),
            project_id: "p".into(),
            environment_id: "e".into(),
            event_id: id.into(),
            event_type: "deploy".into(),
            service_id: String::new(),
            service_name: String::new(),
            service_instance_id: String::new(),
            entity_id: String::new(),
            event_time: T0,
            observed_time: T0,
            trace_id: String::new(),
            span_id: String::new(),
            source_id: "src".into(),
            source_epoch: String::new(),
            source_sequence: Some(u64::MAX),
            attributes: vec![],
            payload_json: "{}".into(),
            batch_id: "b".into(),
            ingested_at: T0,
        }
    }

    #[test]
    fn event_dtos_keep_u64_sequences_as_strings() {
        let d = event_dto(event_row("e1"));
        assert_eq!(d.source_sequence.as_deref(), Some("18446744073709551615"));
        assert_eq!(d.source_id.as_deref(), Some("src"));
        assert_eq!((d.entity_id, d.trace_id, d.source_epoch), (None, None, None));
        let v = serde_json::to_value(event_dto(event_row("e1"))).unwrap();
        assert!(v["source_sequence"].is_string(), "u64 does not fit a JS number");
        let TimelineItem::Event(e) = event_item(event_row("e2")) else { panic!("event") };
        assert_eq!((e.service_id, e.service_name), (None, None));
    }

    fn tl_span(id: &str, status: u8) -> TimelineSpanRow {
        TimelineSpanRow {
            trace_id: "t".into(),
            span_id: id.into(),
            parent_span_id: String::new(),
            service_id: "svc".into(),
            service_name: "svc".into(),
            target_entity_id: String::new(),
            name: "op".into(),
            kind: 2,
            start_time: T0,
            end_time: T0 + 1,
            status_code: status,
            ingested_at: T0,
        }
    }

    fn tl_log(id: &str) -> TimelineLogRow {
        TimelineLogRow {
            log_id: id.into(),
            service_id: "svc".into(),
            service_name: "svc".into(),
            time: T0,
            severity_number: 9,
            trace_id: "t".into(),
            ingested_at: T0,
        }
    }

    #[test]
    fn span_items_flag_only_otlp_error_status() {
        for (status, err) in [(0, false), (1, false), (2, true), (3, false)] {
            let TimelineItem::Span(s) = span_item(tl_span("s", status), "cl") else { panic!("span") };
            assert_eq!(s.is_error, err, "status {status}");
            assert_eq!((s.parent_span_id, s.target_entity_id), (None, None));
        }
        let TimelineItem::Log(l) = log_item(tl_log("l")) else { panic!("log") };
        assert_eq!(l.trace_id.as_deref(), Some("t"));
    }

    #[test]
    fn timeline_truncation_is_reported_per_signal() {
        let spans: Vec<_> = (0..3).map(|i| tl_span(&i.to_string(), 0)).collect();
        let logs: Vec<_> = (0..2).map(|i| tl_log(&i.to_string())).collect();
        let events: Vec<_> = (0..1).map(|i| event_row(&i.to_string())).collect();
        let (items, st, lt, et) = timeline_items(spans.clone(), logs.clone(), events.clone(), 2, "cl");
        assert_eq!((st, lt, et), (true, false, false));
        assert_eq!(items.len(), 2 + 2 + 1);
        let (items, st, lt, et) = timeline_items(spans, logs, events, 1, "cl");
        assert_eq!((st, lt, et), (true, true, false));
        assert_eq!(items.len(), 3);
        let (items, st, lt, et) = timeline_items(vec![], vec![], vec![], 1, "cl");
        assert!(items.is_empty() && !st && !lt && !et);
    }

    fn span_row(id: &str, parent: &str) -> SpanRow {
        SpanRow {
            tenant_id: "t".into(),
            project_id: "p".into(),
            environment_id: "e".into(),
            service_id: "svc".into(),
            service_name: "svc".into(),
            service_namespace: String::new(),
            service_instance_id: String::new(),
            service_version: "1.2.3".into(),
            trace_id: "t".into(),
            span_id: id.into(),
            parent_span_id: parent.into(),
            name: "op".into(),
            kind: 2,
            start_time: T0,
            end_time: T0 + 1_500_000,
            duration_ns: 1_500_000,
            status_code: 2,
            status_message: "boom".into(),
            target_entity_id: String::new(),
            target_name: String::new(),
            target_kind: String::new(),
            attributes: vec![("http.method".into(), "GET".into())],
            resource_attributes: vec![],
            batch_id: "b".into(),
            ingested_at: T0,
        }
    }

    #[test]
    fn missing_parents_are_unique_sorted_and_exclude_recorded_spans() {
        let spans =
            [span_row("a", ""), span_row("b", "a"), span_row("c", "zz"), span_row("d", "zz"), span_row("e", "mm")];
        assert_eq!(missing_parents(&spans), ["mm", "zz"]);
        assert!(missing_parents(&[]).is_empty());
        let d = span_dto(span_row("b", "a"));
        assert_eq!(d.duration_ms, 1.5);
        assert_eq!(d.parent_span_id.as_deref(), Some("a"));
        assert_eq!(d.service_version.as_deref(), Some("1.2.3"));
        assert_eq!((d.service_instance_id, d.target_name), (None, None));
        assert_eq!(span_dto(span_row("a", "")).parent_span_id, None);
    }

    fn svc(id: &str, requests: u64, p95: f64, last: i64) -> ServiceSummaryRow {
        ServiceSummaryRow {
            service_id: id.into(),
            service_name: format!("{id}-name"),
            requests,
            errors: requests / 10,
            p95_ns: p95,
            last_seen: last,
            instances: 2,
            version: String::new(),
        }
    }

    fn log_svc(id: &str, last: i64) -> LogServiceRow {
        LogServiceRow {
            service_id: id.into(),
            service_name: format!("{id}-name"),
            logs: 7,
            error_logs: 1,
            last_seen: last,
        }
    }

    #[test]
    fn services_merge_spans_and_logs() {
        let out = merge_services(
            vec![svc("b", 100, 2_000_000.0, 10), svc("c", 0, f64::NAN, 5), svc("d", 5, f64::INFINITY, 1)],
            vec![log_svc("b", 20), log_svc("a", 3)],
        );
        assert_eq!(out.iter().map(|s| s.service_id.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d"]);
        let [a, b, c, d] = [&out[0], &out[1], &out[2], &out[3]];
        assert_eq!((a.requests, a.logs, a.p95_ms, a.instances), (0, 7, None, 0));
        assert_eq!(a.signals, ["logs"]);
        assert_eq!(a.last_seen_ns.as_deref(), Some("3"));
        assert_eq!(b.signals, ["traces", "logs"]);
        assert_eq!(b.p95_ms, Some(2.0));
        assert_eq!(b.last_seen_ns.as_deref(), Some("20"), "latest of spans and logs");
        assert_eq!((b.logs, b.error_logs), (7, 1));
        assert_eq!(b.version, None, "empty version is null");
        assert_eq!(c.p95_ms, None, "no requests → no p95");
        assert_eq!(d.p95_ms, None, "non-finite p95 is never serialized");
        assert!(serde_json::to_string(&out).is_ok());
    }

    #[test]
    fn topology_nodes_and_edges() {
        let calls = vec![CallEdgeRow {
            source_service_id: "a".into(),
            target_service_id: "b".into(),
            calls: 3,
            errors: 1,
            last_seen: 9,
        }];
        let res = |id: &str, kind: &str| ResourceEdgeRow {
            service_id: "b".into(),
            target_entity_id: id.into(),
            target_name: format!("{id}-name"),
            target_kind: kind.into(),
            calls: 1,
            errors: 0,
            last_seen: 4,
        };
        let resources = vec![res("db1", "database"), res("q1", "queue"), res("x1", "http"), res("a", "database")];
        let (nodes, edges) =
            build_topology(&[svc("a", 1, 1.0, 1)], &[log_svc("b", 1), log_svc("a", 1)], calls, resources, vec![], "c");
        let v = serde_json::to_value(&nodes).unwrap();
        let by_id = |id: &str| v.as_array().unwrap().iter().find(|n| n["id"] == id).cloned().unwrap();
        assert_eq!(nodes.len(), 5, "a, b, db1, q1, x1 (a is not duplicated)");
        assert_eq!(by_id("a")["provenance"], serde_json::to_value(ProvenanceDto::Observed).unwrap());
        assert_eq!(by_id("a")["kind"], serde_json::to_value(EntityKindDto::Service).unwrap(), "observed node wins");
        assert_eq!(by_id("db1")["kind"], serde_json::to_value(EntityKindDto::Database).unwrap());
        assert_eq!(by_id("q1")["kind"], serde_json::to_value(EntityKindDto::Queue).unwrap());
        assert_eq!(by_id("x1")["kind"], serde_json::to_value(EntityKindDto::ExternalDependency).unwrap());
        assert_eq!(by_id("db1")["provenance"], serde_json::to_value(ProvenanceDto::Inferred).unwrap());

        assert_eq!(edges.len(), 5);
        assert_eq!((edges[0].source.as_str(), edges[0].target.as_str(), edges[0].calls), ("a", "b", 3));
        assert!(edges[0].reasons.is_empty(), "observed edges need no inferred reasons");
        for e in &edges[1..] {
            assert_eq!(
                serde_json::to_value(e.provenance).unwrap(),
                serde_json::to_value(ProvenanceDto::Inferred).unwrap()
            );
            assert_eq!(e.reasons.len(), 1);
        }
    }

    #[test]
    fn external_services_and_clients_are_marked_as_outside_the_project() {
        let ext = ResourceEdgeRow {
            service_id: "api".into(),
            target_entity_id: "x1".into(),
            target_name: "api.cdek.ru".into(),
            target_kind: "external".into(),
            calls: 4,
            errors: 1,
            last_seen: 7,
        };
        let clients = vec![
            ExternalClientEdgeRow { service_id: "api".into(), calls: 10, errors: 2, last_seen: 8 },
            ExternalClientEdgeRow { service_id: "gateway".into(), calls: 1, errors: 0, last_seen: 3 },
        ];
        let (nodes, edges) =
            build_topology(&[svc("api", 10, 1.0, 1), svc("gateway", 1, 1.0, 1)], &[], vec![], vec![ext], clients, "cl");
        let node = |id: &str| nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(nodes.len(), 4, "api, gateway, one external service, one shared external clients node");
        assert_eq!((node("x1").kind, node("x1").name.as_str()), (EntityKindDto::ExternalDependency, "api.cdek.ru"));
        assert_eq!((node("cl").kind, node("cl").name.as_str()), (EntityKindDto::ExternalClient, "External clients"));
        assert_eq!(node("cl").provenance, ProvenanceDto::Inferred);

        let edge = |s: &str, t: &str| edges.iter().find(|e| e.source == s && e.target == t).unwrap();
        let out = edge("api", "x1");
        assert_eq!((out.relation, out.calls, out.errors), (RelationKindDto::DependsOn, 4, 1));
        assert!(out.reasons[0].contains("external service"), "{:?}", out.reasons);
        let inb = edge("cl", "api");
        assert_eq!(
            (inb.relation, inb.provenance, inb.calls, inb.errors),
            (RelationKindDto::Calls, ProvenanceDto::Inferred, 10, 2)
        );
        assert!(inb.reasons[0].contains("outside the project"), "{:?}", inb.reasons);
        assert_eq!(edge("cl", "gateway").calls, 1);
    }

    #[test]
    fn external_clients_id_is_stable_per_environment_and_scoped() {
        let scope = |e: &str| Scope {
            tenant_id: controlhub_domain::TenantId::parse("t-1").unwrap(),
            project_id: controlhub_domain::ProjectId::parse("p-1").unwrap(),
            environment_id: controlhub_domain::EnvironmentId::parse(e).unwrap(),
        };
        assert_eq!(external_clients_id(&scope("e-1")), external_clients_id(&scope("e-1")));
        assert_ne!(external_clients_id(&scope("e-1")), external_clients_id(&scope("e-2")));
    }

    #[test]
    fn only_root_server_spans_come_from_external_clients() {
        let item = |kind: u8, parent: &str| {
            let TimelineItem::Span(s) =
                span_item(TimelineSpanRow { kind, parent_span_id: parent.into(), ..tl_span("s", 0) }, "cl")
            else {
                panic!("span")
            };
            s.source_entity_id
        };
        assert_eq!(item(2, "").as_deref(), Some("cl"));
        assert_eq!(item(2, "p"), None, "an instrumented caller exists");
        for kind in [0, 1, 3, 4, 5] {
            assert_eq!(item(kind, ""), None, "kind {kind}");
        }
    }

    fn wm(next: i64, high: i64, received: i64, updated_secs: i64) -> WatermarkRow {
        WatermarkRow {
            partition: 0,
            next_offset: next,
            high_watermark: high,
            last_received_time_ns: received,
            updated_at: time::OffsetDateTime::from_unix_timestamp(updated_secs).unwrap(),
        }
    }

    #[test]
    fn completeness_reflects_processing_state() {
        let none = completeness_from(&[], 0, false);
        assert!(!none.processing_caught_up && none.watermark_ns.is_none());
        assert!(none.notes.iter().any(|n| n.contains("behind")));

        let ok = completeness_from(&[wm(10, 10, 5, 100), wm(11, 10, 6, 200)], 0, true);
        assert!(ok.processing_caught_up && ok.truncated);
        assert_eq!(ok.watermark_ns.as_deref(), Some("100000000000"), "minimum over partitions");
        assert_eq!(ok.notes.len(), 1);
        assert_eq!(ok.sampling, "unknown");

        let behind = completeness_from(&[wm(10, 10, 5, 100), wm(3, 10, 42, 200)], 2, false);
        assert!(!behind.processing_caught_up);
        assert_eq!(behind.watermark_ns.as_deref(), Some("42"), "lagging partition bounds the watermark");
        assert_eq!(behind.unresolved_processing_errors, 2);
        assert!(behind.notes.iter().any(|n| n.contains("2 record(s) failed")));
        assert!(behind.notes.iter().any(|n| n.contains("behind")));
    }
}
