//! Scoped telemetry queries. Every handler resolves the scope through
//! [`authorize`] before touching storage.

use std::collections::{BTreeMap, HashSet};

use axum::Json;
use axum::extract::{Path, Query, State};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use controlhub_domain::{Scope, TraceId};
use controlhub_engine_core::{
    EntityKindDto, EventItem, LogItem, ProvenanceDto, RelationKindDto, SpanItem, TimelineItem,
};
use controlhub_storage::ch::{EventRow, LogQuery, LogRow, TimelineLogRow, TimelineSpanRow, TraceQuery};
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
        let from = parse_ns(&self.from, "from")?.unwrap_or(to - 15 * NS_PER_MIN);
        if from >= to {
            return Err(ApiError::BadRequest("from must be before to".into()));
        }
        if to - from > MAX_RANGE_NS {
            return Err(ApiError::BadRequest("time range may not exceed 7 days".into()));
        }
        Ok(Range { from, to })
    }
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
    let mut caught_up = !wms.is_empty();
    let mut watermark: Option<i64> = None;
    for w in &wms {
        let at = if w.next_offset >= w.high_watermark {
            (w.updated_at.unix_timestamp_nanos()).clamp(0, i64::MAX as i128) as i64
        } else {
            caught_up = false;
            w.last_received_time_ns
        };
        watermark = Some(watermark.map_or(at, |m| m.min(at)));
    }
    let dead = state.pg.count_unresolved_processing_errors(scope.tenant_id.as_str()).await?;
    let mut notes = vec!["Only instrumented components are visible.".to_owned()];
    if !caught_up {
        notes.push("Processing is behind ingestion; recent data may be missing.".into());
    }
    if dead > 0 {
        notes.push(format!("{dead} record(s) failed processing and are not included."));
    }
    Ok(Completeness {
        watermark_ns: watermark.map(ns),
        processing_caught_up: caught_up,
        unresolved_processing_errors: dead,
        truncated,
        sampling: "unknown".into(),
        notes,
    })
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
    Ok(Json(ServicesResponse {
        services: out.into_values().collect(),
        from_ns: ns(r.from),
        to_ns: ns(r.to),
        is_demo,
        completeness: completeness(&state, &scope, false).await?,
    }))
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
    let (services, logs, calls, resources) = tokio::try_join!(
        state.ch.service_summaries(&scope, r.from, r.to),
        state.ch.log_services(&scope, r.from, r.to),
        state.ch.call_edges(&scope, r.from, r.to),
        state.ch.resource_edges(&scope, r.from, r.to),
    )?;
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
            reasons: vec![format!(
                "client spans carry {} semantic-convention attributes for '{}'; the resource itself is not instrumented",
                rsc.target_kind, rsc.target_name
            )],
            calls: rsc.calls,
            errors: rsc.errors,
            last_seen_ns: ns(rsc.last_seen),
        });
    }
    Ok(Json(TopologyResponse {
        nodes: nodes.into_values().collect(),
        edges,
        from_ns: ns(r.from),
        to_ns: ns(r.to),
        completeness: completeness(&state, &scope, false).await?,
    }))
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
    let r = RangeParams { from: q.from.clone(), to: q.to.clone() }.resolve()?;
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let rows = state
        .ch
        .traces(
            &scope,
            &TraceQuery {
                from_ns: r.from,
                to_ns: r.to,
                service_id: q.service_id.clone(),
                only_errors: q.only_errors,
                before: decode_cursor(&q.cursor)?,
                limit: limit + 1,
            },
        )
        .await?;
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
    Ok(Json(TraceListResponse { traces, next_cursor, completeness: completeness(&state, &scope, has_more).await? }))
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
    let (from, to) = (min - NS_PER_MIN, max + NS_PER_MIN);
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
    let ids: HashSet<&str> = spans.iter().map(|s| s.span_id.as_str()).collect();
    let mut missing: Vec<String> = spans
        .iter()
        .filter(|s| !s.parent_span_id.is_empty() && !ids.contains(s.parent_span_id.as_str()))
        .map(|s| s.parent_span_id.clone())
        .collect();
    missing.sort();
    missing.dedup();
    let span_dtos = spans
        .into_iter()
        .take(SPAN_LIMIT as usize)
        .map(|s| SpanDto {
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
        })
        .collect();
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
    let r = RangeParams { from: q.from.clone(), to: q.to.clone() }.resolve()?;
    let limit = q.limit.unwrap_or(200).clamp(1, 1_000);
    if let Some(t) = &q.trace_id {
        TraceId::parse(t.clone()).map_err(|_| ApiError::BadRequest("invalid trace id".into()))?;
    }
    let text = q.q.clone().filter(|s| !s.trim().is_empty());
    if text.as_ref().is_some_and(|s| s.len() > 256) {
        return Err(ApiError::BadRequest("q too long".into()));
    }
    let rows = state
        .ch
        .logs(
            &scope,
            &LogQuery {
                from_ns: r.from,
                to_ns: r.to,
                service_id: q.service_id.clone(),
                min_severity: q.min_severity,
                trace_id: q.trace_id.clone(),
                text,
                before: decode_cursor(&q.cursor)?,
                limit: limit + 1,
            },
        )
        .await?;
    let has_more = rows.len() > limit as usize;
    let logs: Vec<LogDto> = rows.into_iter().take(limit as usize).map(log_dto).collect();
    let next_cursor = if has_more {
        logs.last().and_then(|l| l.time_ns.parse::<i64>().ok().map(|t| encode_cursor(t, &l.id)))
    } else {
        None
    };
    Ok(Json(LogListResponse { logs, next_cursor, completeness: completeness(&state, &scope, has_more).await? }))
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
    let limit = q.limit.unwrap_or(200).clamp(1, 1_000);
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

pub(crate) fn span_item(s: TimelineSpanRow) -> TimelineItem {
    TimelineItem::Span(SpanItem {
        trace_id: s.trace_id,
        span_id: s.span_id,
        parent_span_id: nonempty(s.parent_span_id),
        service_id: s.service_id,
        service_name: s.service_name,
        target_entity_id: nonempty(s.target_entity_id),
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
    let limit = q.limit.unwrap_or(20_000).clamp(1, 100_000);
    let (spans, logs, events) = tokio::try_join!(
        state.ch.timeline_spans(&scope, r.from, r.to, limit + 1),
        state.ch.timeline_logs(&scope, r.from, r.to, limit + 1),
        state.ch.events(&scope, r.from, r.to, None, limit + 1),
    )?;
    let (st, lt, et) = (spans.len() > limit as usize, logs.len() > limit as usize, events.len() > limit as usize);
    let mut items = Vec::with_capacity(spans.len() + logs.len() + events.len());
    items.extend(spans.into_iter().take(limit as usize).map(span_item));
    items.extend(logs.into_iter().take(limit as usize).map(log_item));
    items.extend(events.into_iter().take(limit as usize).map(event_item));
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
