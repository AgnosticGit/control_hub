//! WebSocket live channel for one environment.
//!
//! One bounded catch-up query on connect recovers gaps from disconnects; steady-state updates
//! arrive from the API replica's Kafka consumer through the scoped in-process live hub.
//!
//! Only items of the authorized environment are sent; no tenant-wide stream exists.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use karbird_domain::Scope;
use karbird_engine_core::{InfrastructureSnapshot, TimelineItem};
use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, mpsc, oneshot};
use utoipa::IntoParams;

use super::telemetry::{
    event_dto, event_item, external_clients_id, log_dto, log_item, now_ns, parse_ns, span_dto, span_item,
};
use crate::auth::{CurrentUser, authorize};
use crate::error::{ApiError, ApiResult};
use crate::live_hub::{EdgeKey, LiveItem, LiveSignal, LiveUpdate};
use crate::permissions::Permission;
use crate::state::AppState;

const HEARTBEAT: Duration = Duration::from_secs(15);
const SESSION_RECHECK: Duration = Duration::from_secs(30);
/// Rows ingested within the last `VISIBILITY_MARGIN_NS` are not read yet.
const VISIBILITY_MARGIN_NS: i64 = 3_000_000_000;
/// A fresh subscription starts this far back (overlaps the client's initial HTTP load).
const INITIAL_OVERLAP_NS: i64 = 30_000_000_000;
/// Only items whose event time is within this window are streamed (matches the live window).
const EVENT_LOOKBACK_NS: i64 = 15 * 60_000_000_000;
const CATCHUP_OVERLAP_NS: i64 = 10_000_000_000;
const CATCHUP_LIMIT: u32 = 5_000;
const INFRASTRUCTURE_REFRESH: Duration = Duration::from_secs(10);
const TIMELINE_LOAD_LIMIT: u32 = 50_000;
const TIMELINE_LOAD_MAX_NS: i64 = 7 * 24 * 60 * 60 * 1_000_000_000;
const TIMELINE_CHUNK_ITEMS: usize = 256;
const TIMELINE_MAX_SPLIT_QUERIES: usize = 64;
const TIMELINE_MIN_SPLIT_NS: i64 = 1_000_000_000;

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LiveParams {
    /// Resume cursor from a previous message (ingestion time, decimal nanoseconds).
    pub resume: Option<String>,
    /// Include coalesced infrastructure snapshots while the infrastructure map layer is enabled.
    #[serde(default)]
    pub infrastructure: bool,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage<'a> {
    // Field order matters: the Worker reads `type` and `resume` from the message head.
    Hello {
        resume: String,
        poll_ms: u64,
    },
    Span {
        resume: String,
        storage_cursor: String,
        item: &'a TimelineItem,
        record: &'a super::telemetry::SpanDto,
    },
    Log {
        resume: String,
        storage_cursor: String,
        item: &'a TimelineItem,
        record: &'a super::telemetry::LogDto,
    },
    Event {
        resume: String,
        storage_cursor: String,
        item: &'a TimelineItem,
        record: &'a super::telemetry::EventDto,
    },
    Action {
        resume: String,
        plan: serde_json::Value,
        snapshot: bool,
    },
    EdgeChange {
        resume: String,
        change: &'a karbird_protocol::EdgeChange,
    },
    Topology {
        resume: String,
        snapshot: &'a super::telemetry::TopologyResponse,
    },
    Infrastructure {
        resume: String,
        snapshot: &'a InfrastructureSnapshot,
    },
    Gap {
        resume: String,
        from_ingested_ns: String,
    },
    Heartbeat {
        resume: String,
    },
    TimelineChunk {
        request_id: &'a str,
        sequence: u32,
        from_ns: String,
        to_ns: String,
        items: &'a [TimelineItem],
    },
    TimelineRangeComplete {
        request_id: &'a str,
        sequence: u32,
        from_ns: String,
        to_ns: String,
    },
    TimelineComplete {
        request_id: &'a str,
        truncated: bool,
        completeness: &'a super::telemetry::Completeness,
        logs_source: super::telemetry::LogSource,
        oldest_ns: Option<String>,
    },
    TimelineError {
        request_id: &'a str,
        message: &'a str,
    },
    Error {
        message: &'a str,
    },
}

enum HistoryOutput {
    Chunk {
        request_id: String,
        sequence: u32,
        from_ns: i64,
        to_ns: i64,
        items: Vec<TimelineItem>,
        ack: oneshot::Sender<()>,
    },
    RangeComplete {
        request_id: String,
        sequence: u32,
        from_ns: i64,
        to_ns: i64,
        ack: oneshot::Sender<()>,
    },
    Complete {
        request_id: String,
        truncated: bool,
        completeness: super::telemetry::Completeness,
        logs_source: super::telemetry::LogSource,
        oldest_ns: Option<String>,
    },
    Error {
        request_id: String,
        message: &'static str,
    },
}

#[derive(Debug, Deserialize)]
struct EdgeControl {
    #[serde(rename = "type")]
    kind: String,
    source: Option<String>,
    target: Option<String>,
    relation: Option<String>,
    after_cursor_ns: Option<String>,
    request_id: Option<String>,
    from_ns: Option<String>,
    to_ns: Option<String>,
    limit: Option<u32>,
    sequence: Option<u32>,
    ranges: Option<Vec<TimelineIntervalControl>>,
}

#[derive(Debug, Deserialize)]
struct TimelineIntervalControl {
    from_ns: String,
    to_ns: String,
}

#[derive(Debug, PartialEq, Eq)]
struct TimelineLoad {
    request_id: String,
    from_ns: i64,
    to_ns: i64,
    limit: u32,
    ranges: Vec<(i64, i64)>,
}

impl EdgeControl {
    fn timeline_load(&self) -> Option<TimelineLoad> {
        if self.kind != "timeline_load" {
            return None;
        }
        let request_id = self.request_id.as_ref()?;
        if request_id.is_empty() || request_id.len() > 128 {
            return None;
        }
        let from_ns = self.from_ns.as_deref()?.parse::<i64>().ok()?;
        let to_ns = self.to_ns.as_deref()?.parse::<i64>().ok()?;
        let limit = self.limit?;
        if from_ns < 0
            || to_ns <= from_ns
            || to_ns.saturating_sub(from_ns) > TIMELINE_LOAD_MAX_NS
            || !(1..=TIMELINE_LOAD_LIMIT).contains(&limit)
        {
            return None;
        }
        let ranges = match &self.ranges {
            Some(ranges) if ranges.is_empty() || ranges.len() > 64 => return None,
            Some(ranges) => {
                let mut parsed = Vec::with_capacity(ranges.len());
                let mut prior_to = from_ns;
                for range in ranges {
                    let from = range.from_ns.parse::<i64>().ok()?;
                    let to = range.to_ns.parse::<i64>().ok()?;
                    if from < from_ns || to > to_ns || from < prior_to || to <= from {
                        return None;
                    }
                    parsed.push((from, to));
                    prior_to = to;
                }
                parsed
            }
            None => vec![(from_ns, to_ns)],
        };
        Some(TimelineLoad { request_id: request_id.clone(), from_ns, to_ns, limit, ranges })
    }

    fn timeline_ack(&self) -> Option<(String, u32)> {
        let request_id = self.request_id.as_ref()?;
        if self.kind != "timeline_ack" || request_id.is_empty() || request_id.len() > 128 {
            return None;
        }
        Some((request_id.clone(), self.sequence?))
    }

    fn timeline_cancel(&self) -> Option<&str> {
        let request_id = self.request_id.as_deref()?;
        (self.kind == "timeline_cancel" && !request_id.is_empty() && request_id.len() <= 128).then_some(request_id)
    }

    fn edge(&self) -> Option<(String, String, String, i64)> {
        let source = self.source.as_ref()?.trim();
        let target = self.target.as_ref()?.trim();
        let relation = self.relation.as_deref()?;
        if source.is_empty() || target.is_empty() || source.len() > 256 || target.len() > 256 || source == target {
            return None;
        }
        if !matches!(relation, "calls" | "depends_on") {
            return None;
        }
        let after = self.after_cursor_ns.as_deref().unwrap_or("0").parse::<i64>().ok()?;
        if after < 0 {
            return None;
        }
        Some((source.to_owned(), target.to_owned(), relation.to_owned(), after))
    }
}

/// Browsers always send `Origin` on WebSocket handshakes; requiring an allowed one blocks
/// cross-site WebSocket hijacking with the session cookie.
fn require_origin(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        Some(o) if state.config.allowed_origins.iter().any(|a| a == o) => Ok(()),
        _ => Err(ApiError::Forbidden("origin not allowed")),
    }
}

/// A client-supplied resume cursor is clamped to the live window, so it can neither request
/// history older than the window nor skip ahead of now.
fn start_cursor(resume: &Option<String>, now: i64) -> ApiResult<i64> {
    let floor = now - EVENT_LOOKBACK_NS;
    Ok(parse_ns(resume.as_deref(), "resume")?.map_or(now - INITIAL_OVERLAP_NS, |r| r.clamp(floor, now)))
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/live", tag = "telemetry",
    description = "WebSocket upgrade. Server messages: hello, span, log, event, infrastructure, topology, gap, heartbeat, error (JSON text frames).",
    params(("project_id" = String, Path), ("environment_id" = String, Path), LiveParams),
    responses((status = 101, description = "Switching protocols"), (status = 403, body = crate::error::ErrorBody),
        (status = 404, body = crate::error::ErrorBody), (status = 429, body = crate::error::ErrorBody)))]
pub async fn live(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path((project_id, environment_id)): Path<(String, String)>,
    Query(q): Query<LiveParams>,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    if user.is_session() {
        require_origin(&state, &headers)?;
    }
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Permission::TelemetryView).await?;
    // Action plans have a separate permission boundary from telemetry. A live telemetry socket
    // may still connect, but only an `actions.view` principal receives this side channel.
    let actions_enabled = authorize(&state, &user, &project_id, &environment_id, Permission::ActionsView).await.is_ok();
    let cursor = start_cursor(&q.resume, now_ns())?;
    let permit = state.live_slots.clone().try_acquire_owned().map_err(|_| ApiError::TooManyRequests)?;
    let infrastructure = q.infrastructure;
    Ok(ws.max_message_size(64 * 1024).on_upgrade(move |socket| {
        run(socket, state, scope, cursor, LiveSession { user, _permit: permit, infrastructure, actions_enabled })
    }))
}

struct LiveSession {
    user: CurrentUser,
    _permit: OwnedSemaphorePermit,
    infrastructure: bool,
    actions_enabled: bool,
}

async fn send(tx: &mut futures::stream::SplitSink<WebSocket, Message>, msg: &ServerMessage<'_>) -> bool {
    match serde_json::to_string(msg) {
        Ok(text) => tx.send(Message::Text(Utf8Bytes::from(text))).await.is_ok(),
        Err(_) => false,
    }
}

async fn send_item(tx: &mut futures::stream::SplitSink<WebSocket, Message>, item: &LiveItem) -> bool {
    let resume = item.cursor_ns.to_string();
    let storage_cursor = item.storage_cursor_ns.to_string();
    match item.signal.as_ref() {
        Some(LiveSignal::Span(record)) => {
            send(tx, &ServerMessage::Span { resume, storage_cursor, item: &item.item, record }).await
        }
        Some(LiveSignal::Log(record)) => {
            send(tx, &ServerMessage::Log { resume, storage_cursor, item: &item.item, record }).await
        }
        Some(LiveSignal::Event(record)) => {
            send(tx, &ServerMessage::Event { resume, storage_cursor, item: &item.item, record }).await
        }
        None => false,
    }
}

async fn send_history_chunk(
    output: &mpsc::Sender<HistoryOutput>,
    request_id: &str,
    sequence: &mut u32,
    from_ns: i64,
    to_ns: i64,
    items: Vec<TimelineItem>,
) -> Result<(), ()> {
    let (ack, received) = oneshot::channel();
    output
        .send(HistoryOutput::Chunk {
            request_id: request_id.to_owned(),
            sequence: *sequence,
            from_ns,
            to_ns,
            items,
            ack,
        })
        .await
        .map_err(|_| ())?;
    received.await.map_err(|_| ())?;
    *sequence = sequence.checked_add(1).ok_or(())?;
    Ok(())
}

async fn send_history_range_complete(
    output: &mpsc::Sender<HistoryOutput>,
    request_id: &str,
    sequence: &mut u32,
    from_ns: i64,
    to_ns: i64,
) -> Result<(), ()> {
    let (ack, received) = oneshot::channel();
    output
        .send(HistoryOutput::RangeComplete {
            request_id: request_id.to_owned(),
            sequence: *sequence,
            from_ns,
            to_ns,
            ack,
        })
        .await
        .map_err(|_| ())?;
    received.await.map_err(|_| ())?;
    *sequence = sequence.checked_add(1).ok_or(())?;
    Ok(())
}

fn split_timeline_interval(from_ns: i64, to_ns: i64, query_count: usize, pending_count: usize) -> Option<(i64, i64)> {
    if to_ns.saturating_sub(from_ns) <= TIMELINE_MIN_SPLIT_NS
        || query_count + pending_count + 2 > TIMELINE_MAX_SPLIT_QUERIES
    {
        return None;
    }
    let middle = from_ns + (to_ns - from_ns) / 2;
    (middle > from_ns && middle < to_ns).then_some((from_ns, middle))
}

async fn stream_timeline_history(
    state: AppState,
    scope: Scope,
    load: TimelineLoad,
    output: mpsc::Sender<HistoryOutput>,
) {
    let mut pending = load.ranges.iter().copied().rev().collect::<VecDeque<_>>();
    let mut sequence = 0_u32;
    let mut query_count = 0_usize;
    let mut truncated = false;
    let mut logs_source = None;

    while let Some((from_ns, to_ns)) = pending.pop_back() {
        if query_count >= TIMELINE_MAX_SPLIT_QUERIES {
            truncated = true;
            break;
        }
        query_count += 1;
        let range = super::telemetry::Range { from: from_ns, to: to_ns };
        let data = match super::telemetry::timeline_data_for_scope(&state, &scope, range, load.limit).await {
            Ok(data) => data,
            Err(error) => {
                tracing::warn!(%error, request_id = %load.request_id, from_ns, to_ns, "timeline WebSocket query failed");
                let _ = output
                    .send(HistoryOutput::Error {
                        request_id: load.request_id,
                        message: "timeline history query failed",
                    })
                    .await;
                return;
            }
        };

        let interval_truncated = data.spans_truncated || data.logs_truncated || data.events_truncated;
        if interval_truncated
            && let Some((left_from, left_to)) = split_timeline_interval(from_ns, to_ns, query_count, pending.len())
        {
            pending.push_back((left_to, to_ns));
            pending.push_back((left_from, left_to));
            continue;
        }

        truncated |= interval_truncated;
        logs_source = Some(match logs_source {
            None => data.logs_source,
            Some(source) if source == data.logs_source => source,
            Some(_) => super::telemetry::LogSource::Mixed,
        });
        for items in data.items.chunks(TIMELINE_CHUNK_ITEMS) {
            if send_history_chunk(&output, &load.request_id, &mut sequence, from_ns, to_ns, items.to_vec())
                .await
                .is_err()
            {
                return;
            }
        }
        if !interval_truncated
            && send_history_range_complete(&output, &load.request_id, &mut sequence, from_ns, to_ns).await.is_err()
        {
            return;
        }
    }

    let completeness =
        match super::telemetry::timeline_completeness_for_scope(&state, &scope, truncated, load.from_ns).await {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, request_id = %load.request_id, "timeline WebSocket completeness query failed");
                let _ = output
                    .send(HistoryOutput::Error {
                        request_id: load.request_id,
                        message: "timeline completeness unavailable",
                    })
                    .await;
                return;
            }
        };
    let oldest_ns = match super::telemetry::earliest_timeline_time_ns(&state, &scope).await {
        Ok(value) => value.map(|time| time.to_string()),
        Err(error) => {
            tracing::warn!(%error, request_id = %load.request_id, "timeline oldest boundary query failed");
            let _ = output
                .send(HistoryOutput::Error { request_id: load.request_id, message: "timeline boundary unavailable" })
                .await;
            return;
        }
    };
    let _ = output
        .send(HistoryOutput::Complete {
            request_id: load.request_id,
            truncated,
            completeness,
            logs_source: logs_source.unwrap_or(super::telemetry::LogSource::Hot),
            oldest_ns,
        })
        .await;
}

async fn run(socket: WebSocket, state: AppState, scope: Scope, mut cursor: i64, session: LiveSession) {
    let LiveSession { user, _permit, infrastructure, actions_enabled } = session;
    let (mut tx, mut rx) = socket.split();
    let mut last_sent = Instant::now();
    let mut last_session_check = Instant::now();
    let (history_tx, mut history_rx) = mpsc::channel::<HistoryOutput>(1);
    let mut history_task: Option<(String, tokio::task::JoinHandle<()>)> = None;
    let mut history_acks = HashMap::<(String, u32), oneshot::Sender<()>>::new();
    let clients_id = external_clients_id(&scope);
    let (replay, mut updates, subscriber_id) =
        match state.live_hub.subscribe_with_controls(scope.clone(), cursor, actions_enabled) {
            Ok(value) => value,
            Err(error) => {
                let _ = send(&mut tx, &ServerMessage::Error { message: "live channel unavailable" }).await;
                tracing::warn!(%error, "live subscription rejected");
                return;
            }
        };
    let catchup_after = cursor.saturating_sub(CATCHUP_OVERLAP_NS);
    if !send(&mut tx, &ServerMessage::Hello { resume: catchup_after.to_string(), poll_ms: 0 }).await {
        return;
    }
    // One bounded catch-up per socket handles API restarts and reconnects. The subscription is
    // installed first, so Kafka items arriving during these queries remain queued for delivery.
    let upto = now_ns().saturating_sub(VISIBILITY_MARGIN_NS);
    if upto > catchup_after {
        let event_from = upto.saturating_sub(EVENT_LOOKBACK_NS);
        let mut offset = 0_u32;
        loop {
            match tokio::try_join!(
                state.ch.live_spans(&scope, event_from, catchup_after, upto, CATCHUP_LIMIT, offset),
                state.ch.live_logs(&scope, event_from, catchup_after, upto, CATCHUP_LIMIT, offset),
                state.ch.live_events(&scope, event_from, catchup_after, upto, CATCHUP_LIMIT, offset),
            ) {
                Ok((spans, logs, events)) => {
                    let has_more = spans.len() == CATCHUP_LIMIT as usize
                        || logs.len() == CATCHUP_LIMIT as usize
                        || events.len() == CATCHUP_LIMIT as usize;
                    let mut items = Vec::with_capacity(spans.len() + logs.len() + events.len());
                    items.extend(spans.into_iter().map(|row| {
                        let cursor_ns = row.ingested_at;
                        let timeline = karbird_storage::ch::TimelineSpanRow {
                            trace_id: row.trace_id.clone(),
                            span_id: row.span_id.clone(),
                            parent_span_id: row.parent_span_id.clone(),
                            service_id: row.service_id.clone(),
                            service_name: row.service_name.clone(),
                            target_entity_id: row.target_entity_id.clone(),
                            name: row.name.clone(),
                            kind: row.kind,
                            start_time: row.start_time,
                            end_time: row.end_time,
                            status_code: row.status_code,
                            ingested_at: row.ingested_at,
                        };
                        let signal = LiveSignal::Span(span_dto(row));
                        LiveItem {
                            cursor_ns,
                            storage_cursor_ns: cursor_ns,
                            item: span_item(timeline, &clients_id),
                            signal: Some(signal),
                        }
                    }));
                    items.extend(logs.into_iter().map(|row| {
                        let cursor_ns = row.ingested_at;
                        let timeline = karbird_storage::ch::TimelineLogRow {
                            log_id: row.log_id.clone(),
                            service_id: row.service_id.clone(),
                            service_name: row.service_name.clone(),
                            time: row.time,
                            severity_number: row.severity_number,
                            trace_id: row.trace_id.clone(),
                            ingested_at: row.ingested_at,
                        };
                        let signal = LiveSignal::Log(log_dto(row));
                        LiveItem {
                            cursor_ns,
                            storage_cursor_ns: cursor_ns,
                            item: log_item(timeline),
                            signal: Some(signal),
                        }
                    }));
                    items.extend(events.into_iter().map(|row| {
                        let cursor_ns = row.ingested_at;
                        let signal = LiveSignal::Event(event_dto(row.clone()));
                        LiveItem {
                            cursor_ns,
                            storage_cursor_ns: cursor_ns,
                            item: event_item(row),
                            signal: Some(signal),
                        }
                    }));
                    items.sort_by_key(|item| item.cursor_ns);
                    let ok = if items.is_empty() {
                        true
                    } else {
                        let mut sent = true;
                        for item in &items {
                            if !send_item(&mut tx, item).await {
                                sent = false;
                                break;
                            }
                        }
                        sent
                    };
                    if !ok {
                        return;
                    }
                    if !has_more {
                        break;
                    }
                    offset = offset.saturating_add(CATCHUP_LIMIT);
                    if offset == u32::MAX {
                        let _ =
                            send(&mut tx, &ServerMessage::Error { message: "live recovery range is too large" }).await;
                        return;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "live catch-up query failed");
                    let _ = send(&mut tx, &ServerMessage::Error { message: "live catch-up unavailable" }).await;
                    return;
                }
            }
        }
        cursor = cursor.max(upto);
    }
    let mut pending = replay.into_iter().filter(|item| item.cursor_ns > cursor).collect::<Vec<_>>();
    pending.sort_by_key(|item| item.cursor_ns);
    if !pending.is_empty() {
        cursor = pending.iter().map(|item| item.cursor_ns).max().unwrap_or(cursor);
        for item in &pending {
            if !send_item(&mut tx, item).await {
                return;
            }
        }
        last_sent = Instant::now();
    }
    // The snapshot is sent over this same authorized socket; later transactional notifications
    // are queued while it is loaded, then delivered as individual action frames.
    if actions_enabled {
        match state.pg.list_action_plans(&scope, 100).await {
            Ok(plans) => {
                for row in plans {
                    let plan: super::actions::ActionPlanDto = row.into();
                    match serde_json::to_value(plan) {
                        Ok(plan) => {
                            if !send(
                                &mut tx,
                                &ServerMessage::Action { resume: cursor.to_string(), plan, snapshot: true },
                            )
                            .await
                            {
                                return;
                            }
                        }
                        Err(error) => tracing::warn!(%error, "could not encode action snapshot item"),
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "live action snapshot failed"),
        }
    }
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut infrastructure_refresh =
        tokio::time::interval_at(tokio::time::Instant::now() + INFRASTRUCTURE_REFRESH, INFRASTRUCTURE_REFRESH);
    infrastructure_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut topology_refresh =
        tokio::time::interval_at(tokio::time::Instant::now() + INFRASTRUCTURE_REFRESH, INFRASTRUCTURE_REFRESH);
    topology_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_topology_update: Option<String> = None;
    if infrastructure {
        let snapshot = super::infrastructure::load_snapshot_coalesced(&state, &scope, now_ns()).await;
        match snapshot {
            Ok(snapshot) => {
                if !send(&mut tx, &ServerMessage::Infrastructure { resume: cursor.to_string(), snapshot: &snapshot })
                    .await
                {
                    return;
                }
                last_sent = Instant::now();
            }
            Err(error) => tracing::warn!(%error, "initial live infrastructure snapshot failed"),
        }
    }
    loop {
        tokio::select! {
            incoming = rx.next() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<EdgeControl>(&text) {
                        Ok(control) if control.kind == "unsubscribe_edge" => {
                            state.live_hub.select_edge(subscriber_id, None);
                        }
                        Ok(control) if control.kind == "timeline_load" => {
                            let Some(load) = control.timeline_load() else {
                                let id = control.request_id.as_deref().filter(|id| !id.is_empty() && id.len() <= 128).unwrap_or("invalid");
                                if !send(&mut tx, &ServerMessage::TimelineError { request_id: id, message: "invalid timeline request" }).await { break; }
                                continue;
                            };
                            if history_task.is_some() {
                                if !send(&mut tx, &ServerMessage::TimelineError { request_id: &load.request_id, message: "another timeline request is active" }).await { break; }
                                continue;
                            }
                            let request_id = load.request_id.clone();
                            let task_state = state.clone();
                            let task_scope = scope.clone();
                            let task_output = history_tx.clone();
                            let task = tokio::spawn(async move {
                                stream_timeline_history(task_state, task_scope, load, task_output).await;
                            });
                            history_task = Some((request_id, task));
                        }
                        Ok(control) if control.kind == "timeline_ack" => {
                            if let Some((request_id, sequence)) = control.timeline_ack()
                                && let Some(ack) = history_acks.remove(&(request_id, sequence))
                            {
                                let _ = ack.send(());
                            }
                        }
                        Ok(control) if control.kind == "timeline_cancel" => {
                            if let Some(request_id) = control.timeline_cancel()
                                && history_task.as_ref().is_some_and(|(active, _)| active == request_id)
                            {
                                if let Some((_, task)) = history_task.take() { task.abort(); }
                                history_acks.retain(|(active, _), _| active != request_id);
                            }
                        }
                        Ok(control) if control.kind == "subscribe_edge" => {
                            let Some((source, target, relation, after)) = control.edge() else {
                                let _ = send(&mut tx, &ServerMessage::Error { message: "invalid edge subscription" }).await;
                                continue;
                            };
                            state.live_hub.select_edge(subscriber_id, Some(EdgeKey { source: source.clone(), target: target.clone(), relation: relation.clone() }));
                            // The initial HTTP chart is the baseline. Durable replay is loaded from
                            // ClickHouse facts; the edge consumer's live messages queue concurrently.
                            let storage_relation = if relation == "calls" && source == clients_id { "external_clients" } else { relation.as_str() };
                            let limit = 5_000;
                            let mut offset = 0_u32;
                            loop {
                                let changes = match state.ch.edge_change_replay(
                                    &scope, &source, &target, storage_relation, after,
                                    karbird_storage::ch::EdgeReplayPage { limit, offset },
                                ).await {
                                    Ok(changes) => changes,
                                    Err(error) => {
                                        tracing::warn!(%error, offset, "edge replay query failed");
                                        let _ = send(&mut tx, &ServerMessage::Error { message: "edge replay unavailable; reconnect to retry" }).await;
                                        break;
                                    }
                                };
                                let page_len = changes.len();
                                for row in changes {
                                    let change = karbird_protocol::EdgeChange {
                                        id: format!("{}:{}:{}:{}:{}", row.trace_id, row.span_id, relation, source, target),
                                        tenant_id: scope.tenant_id.to_string(), project_id: scope.project_id.to_string(),
                                        environment_id: scope.environment_id.to_string(), source: source.clone(), target: target.clone(),
                                        relation: relation.clone(), trace_id: row.trace_id, span_id: row.span_id,
                                        time_ns: row.time_ns.to_string(), storage_cursor_ns: row.storage_cursor_ns.to_string(),
                                        request_bytes: row.request_bytes, response_bytes: row.response_bytes,
                                    };
                                    if !send(&mut tx, &ServerMessage::EdgeChange { resume: change.storage_cursor_ns.clone(), change: &change }).await { return; }
                                }
                                if page_len < limit as usize { break; }
                                let Some(next_offset) = offset.checked_add(limit) else {
                                    let _ = send(&mut tx, &ServerMessage::Error { message: "edge replay limit exceeded" }).await;
                                    break;
                                };
                                offset = next_offset;
                            }
                        }
                        _ => {}
                    }
                }
                // Client binary frames and pings are not application controls.
                Some(Ok(_)) => {}
            },
            update = updates.recv() => {
                let Some(update) = update else {
                    let _ = send(&mut tx, &ServerMessage::Gap { resume: cursor.to_string(), from_ingested_ns: cursor.to_string() }).await;
                    break;
                };
                let sent = match update {
                    LiveUpdate::Telemetry(item) => {
                        cursor = cursor.max(item.cursor_ns);
                        send_item(&mut tx, &item).await
                    }
                    LiveUpdate::Action(plan) if actions_enabled => send(&mut tx, &ServerMessage::Action { resume: cursor.to_string(), plan, snapshot: false }).await,
                    LiveUpdate::Action(_) => true,
                    LiveUpdate::Edge(change) => send(&mut tx, &ServerMessage::EdgeChange { resume: change.storage_cursor_ns.clone(), change: &change }).await,
                };
                if !sent { break; }
                last_sent = Instant::now();
            }
            history = history_rx.recv() => {
                let Some(history) = history else { break; };
                let sent = match history {
                    HistoryOutput::Chunk { request_id, sequence, from_ns, to_ns, items, ack } => {
                        history_acks.insert((request_id.clone(), sequence), ack);
                        send(&mut tx, &ServerMessage::TimelineChunk { request_id: &request_id, sequence, from_ns: from_ns.to_string(), to_ns: to_ns.to_string(), items: &items }).await
                    }
                    HistoryOutput::RangeComplete { request_id, sequence, from_ns, to_ns, ack } => {
                        history_acks.insert((request_id.clone(), sequence), ack);
                        send(&mut tx, &ServerMessage::TimelineRangeComplete { request_id: &request_id, sequence, from_ns: from_ns.to_string(), to_ns: to_ns.to_string() }).await
                    }
                    HistoryOutput::Complete { request_id, truncated, completeness, logs_source, oldest_ns } => {
                        let result = send(&mut tx, &ServerMessage::TimelineComplete { request_id: &request_id, truncated, completeness: &completeness, logs_source, oldest_ns }).await;
                        if history_task.as_ref().is_some_and(|(active, _)| active == &request_id) { history_task = None; }
                        result
                    }
                    HistoryOutput::Error { request_id, message } => {
                        let result = send(&mut tx, &ServerMessage::TimelineError { request_id: &request_id, message }).await;
                        if history_task.as_ref().is_some_and(|(active, _)| active == &request_id) {
                            if let Some((_, task)) = history_task.take() { task.abort(); }
                            history_acks.retain(|(active, _), _| active != &request_id);
                        }
                        result
                    }
                };
                if !sent { break; }
                last_sent = Instant::now();
            }
            _ = infrastructure_refresh.tick(), if infrastructure => {
                let at = now_ns();
                let snapshot = super::infrastructure::load_snapshot_coalesced(&state, &scope, at).await;
                match snapshot {
                    Ok(snapshot) => {
                        if !send(&mut tx, &ServerMessage::Infrastructure { resume: cursor.to_string(), snapshot: &snapshot }).await { break; }
                        last_sent = Instant::now();
                    }
                    Err(error) => tracing::warn!(%error, "live infrastructure snapshot failed"),
                }
            }
            _ = topology_refresh.tick() => {
                let to = now_ns();
                let snapshot = state.live_hub.topology_snapshot(&scope, || async {
                    crate::topology::read(
                        &state,
                        &scope,
                        super::telemetry::Range { from: to.saturating_sub(EVENT_LOOKBACK_NS), to },
                        true,
                    ).await
                }).await;
                match snapshot {
                    Ok(snapshot) if last_topology_update.as_deref() != Some(snapshot.updated_at_ns.as_str()) => {
                        if !send(&mut tx, &ServerMessage::Topology { resume: cursor.to_string(), snapshot: &snapshot }).await { break; }
                        last_topology_update = Some(snapshot.updated_at_ns.clone());
                        last_sent = Instant::now();
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "live topology snapshot failed"),
                }
            }
            _ = heartbeat.tick() => {
                if last_session_check.elapsed() >= SESSION_RECHECK {
                    last_session_check = Instant::now();
                    if !user.still_active(&state).await.unwrap_or(false) {
                        let _ = send(&mut tx, &ServerMessage::Error { message: "authorization ended" }).await;
                        break;
                    }
                }
                if last_sent.elapsed() >= HEARTBEAT {
                    if !send(&mut tx, &ServerMessage::Heartbeat { resume: cursor.to_string() }).await { break; }
                    last_sent = Instant::now();
                }
            }
        }
    }
    if let Some((_, task)) = history_task {
        task.abort();
    }
    history_acks.clear();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn edge_controls_validate_scope_keys_and_decimal_resume_cursors() {
        let control: EdgeControl = serde_json::from_str(
            r#"{"type":"subscribe_edge","source":"gateway","target":"orders","relation":"calls","after_cursor_ns":"1790000000000000000"}"#,
        ).unwrap();
        assert_eq!(
            control.edge(),
            Some(("gateway".into(), "orders".into(), "calls".into(), 1_790_000_000_000_000_000))
        );

        for invalid in [
            r#"{"type":"subscribe_edge","source":" ","target":"orders","relation":"calls"}"#,
            r#"{"type":"subscribe_edge","source":"gateway","target":"orders","relation":"unknown"}"#,
            r#"{"type":"subscribe_edge","source":"gateway","target":"orders","relation":"calls","after_cursor_ns":"-1"}"#,
            r#"{"type":"subscribe_edge","source":"gateway","target":"orders","relation":"calls","after_cursor_ns":"NaN"}"#,
        ] {
            let parsed: EdgeControl = serde_json::from_str(invalid).unwrap();
            assert_eq!(parsed.edge(), None, "{invalid}");
        }
    }

    #[test]
    fn timeline_load_controls_validate_range_limit_and_request_identity() {
        let control: EdgeControl = serde_json::from_str(
            r#"{"type":"timeline_load","request_id":"range-1","from_ns":"100","to_ns":"200","limit":50000}"#,
        )
        .unwrap();
        assert_eq!(
            control.timeline_load(),
            Some(TimelineLoad {
                request_id: "range-1".into(),
                from_ns: 100,
                to_ns: 200,
                limit: 50_000,
                ranges: vec![(100, 200)],
            })
        );

        for invalid in [
            r#"{"type":"timeline_load","request_id":"range-1","from_ns":"x","to_ns":"200","limit":50000}"#,
            r#"{"type":"timeline_load","request_id":"range-1","from_ns":"200","to_ns":"100","limit":50000}"#,
            r#"{"type":"timeline_load","request_id":"range-1","from_ns":"100","to_ns":"200","limit":0}"#,
            r#"{"type":"timeline_load","request_id":"","from_ns":"100","to_ns":"200","limit":50000}"#,
            r#"{"type":"timeline_load","request_id":"range-1","from_ns":"100","to_ns":"200","limit":50001}"#,
        ] {
            let parsed: EdgeControl = serde_json::from_str(invalid).unwrap();
            assert_eq!(parsed.timeline_load(), None, "{invalid}");
        }

        let partial: EdgeControl = serde_json::from_str(
            r#"{"type":"timeline_load","request_id":"range-2","from_ns":"100","to_ns":"200","limit":200,"ranges":[{"from_ns":"100","to_ns":"140"},{"from_ns":"160","to_ns":"200"}]}"#,
        ).unwrap();
        assert_eq!(partial.timeline_load().map(|load| load.ranges), Some(vec![(100, 140), (160, 200)]));
        let overlap: EdgeControl = serde_json::from_str(
            r#"{"type":"timeline_load","request_id":"range-2","from_ns":"100","to_ns":"200","limit":200,"ranges":[{"from_ns":"100","to_ns":"170"},{"from_ns":"160","to_ns":"200"}]}"#,
        ).unwrap();
        assert_eq!(overlap.timeline_load(), None);

        let ack: EdgeControl =
            serde_json::from_str(r#"{"type":"timeline_ack","request_id":"range-1","sequence":4}"#).unwrap();
        assert_eq!(ack.timeline_ack(), Some(("range-1".into(), 4)));
        let cancel: EdgeControl = serde_json::from_str(r#"{"type":"timeline_cancel","request_id":"range-1"}"#).unwrap();
        assert_eq!(cancel.timeline_cancel(), Some("range-1"));
    }

    #[test]
    fn capped_history_ranges_split_only_within_the_query_budget() {
        assert_eq!(split_timeline_interval(0, 10_000_000_000, 1, 0), Some((0, 5_000_000_000)));
        assert_eq!(split_timeline_interval(0, TIMELINE_MIN_SPLIT_NS, 1, 0), None);
        assert_eq!(split_timeline_interval(0, 10_000_000_000, TIMELINE_MAX_SPLIT_QUERIES - 1, 0), None);
    }

    #[test]
    fn timeline_history_frames_keep_the_wire_contract() {
        let chunk = serde_json::to_string(&ServerMessage::TimelineChunk {
            request_id: "load-1",
            sequence: 7,
            from_ns: "100".into(),
            to_ns: "200".into(),
            items: &[],
        })
        .unwrap();
        assert_eq!(
            chunk,
            r#"{"type":"timeline_chunk","request_id":"load-1","sequence":7,"from_ns":"100","to_ns":"200","items":[]}"#
        );

        let complete = serde_json::to_string(&ServerMessage::TimelineRangeComplete {
            request_id: "load-1",
            sequence: 8,
            from_ns: "100".into(),
            to_ns: "200".into(),
        })
        .unwrap();
        assert_eq!(
            complete,
            r#"{"type":"timeline_range_complete","request_id":"load-1","sequence":8,"from_ns":"100","to_ns":"200"}"#
        );

        let final_frame = serde_json::to_string(&ServerMessage::TimelineComplete {
            request_id: "load-1",
            truncated: false,
            completeness: &super::super::telemetry::Completeness {
                watermark_ns: None,
                processing_caught_up: true,
                unresolved_processing_errors: 0,
                truncated: false,
                sampling: "none".into(),
                notes: vec![],
            },
            logs_source: super::super::telemetry::LogSource::Hot,
            oldest_ns: Some("100".into()),
        })
        .unwrap();
        assert!(final_frame.contains(r#""oldest_ns":"100""#), "{final_frame}");
    }

    #[tokio::test]
    async fn history_chunk_waits_for_ack_before_advancing_sequence() {
        let (output, mut receiver) = mpsc::channel(1);
        let mut sequence = 0;
        {
            let transfer = send_history_chunk(&output, "load-ack", &mut sequence, 100, 200, vec![]);
            tokio::pin!(transfer);
            assert!(futures::poll!(transfer.as_mut()).is_pending());

            let Some(HistoryOutput::Chunk { sequence: sent_sequence, ack, .. }) = receiver.recv().await else {
                panic!("expected a history chunk");
            };
            assert_eq!(sent_sequence, 0);
            assert!(futures::poll!(transfer.as_mut()).is_pending());
            let _ = ack.send(());
            assert!(transfer.as_mut().await.is_ok());
        }
        assert_eq!(sequence, 1);
    }

    #[test]
    fn each_live_record_is_one_websocket_message_accepted_by_the_engine() {
        let item = TimelineItem::Log(karbird_engine_core::LogItem {
            id: "l1".into(),
            service_id: "svc_a".into(),
            service_name: "a".into(),
            time_ns: 1_790_000_000_000_000_000,
            severity: 9,
            trace_id: None,
        });
        let record = super::super::telemetry::LogDto {
            id: "l1".into(),
            service_id: "svc_a".into(),
            service_name: "a".into(),
            time_ns: "1790000000000000000".into(),
            observed_time_ns: "1790000000000000000".into(),
            severity_number: 9,
            severity_text: "ERROR".into(),
            body: "failed".into(),
            trace_id: None,
            span_id: None,
            attributes: Default::default(),
            template_id: "0".into(),
        };
        let text = serde_json::to_string(&ServerMessage::Log {
            resume: "42".into(),
            storage_cursor: "40".into(),
            item: &item,
            record: &record,
        })
        .unwrap();
        assert!(
            text.starts_with(r#"{"type":"log","resume":"42","storage_cursor":"40","item":{"type":"log""#),
            "{text}"
        );
        let parsed = serde_json::from_str::<serde_json::Value>(&text).unwrap();
        assert!(parsed.get("items").is_none());
        assert!(parsed.get("signals").is_none());
        let mut e = karbird_engine_core::Engine::new(Default::default());
        e.apply_batch_json(&format!(r#"{{"items":[{}]}}"#, serde_json::to_string(&item).unwrap())).unwrap();
        assert_eq!(e.item_count(), 1);
    }

    #[test]
    fn control_messages_have_the_documented_shape() {
        let j = |m: &ServerMessage<'_>| serde_json::to_string(m).unwrap();
        assert_eq!(
            j(&ServerMessage::Hello { resume: "1".into(), poll_ms: 0 }),
            r#"{"type":"hello","resume":"1","poll_ms":0}"#
        );
        assert_eq!(
            j(&ServerMessage::Gap { resume: "9".into(), from_ingested_ns: "3".into() }),
            r#"{"type":"gap","resume":"9","from_ingested_ns":"3"}"#
        );
        assert_eq!(j(&ServerMessage::Heartbeat { resume: "5".into() }), r#"{"type":"heartbeat","resume":"5"}"#);
        assert_eq!(
            j(&ServerMessage::Error { message: "authorization ended" }),
            r#"{"type":"error","message":"authorization ended"}"#
        );
    }

    #[test]
    fn live_frames_include_full_signal_records_and_infrastructure_snapshots() {
        let record = LiveSignal::Event(super::super::telemetry::EventDto {
            id: "event-1".into(),
            event_type: "order.created".into(),
            service_id: "orders".into(),
            service_name: "orders".into(),
            entity_id: None,
            time_ns: "10".into(),
            observed_time_ns: "11".into(),
            trace_id: None,
            span_id: None,
            source_id: None,
            source_epoch: None,
            source_sequence: Some("7".into()),
            attributes: Default::default(),
        });
        let event = match &record {
            LiveSignal::Event(dto) => dto,
            _ => unreachable!(),
        };
        let event_item = TimelineItem::Event(karbird_engine_core::EventItem {
            id: "event-1".into(),
            event_type: "order.created".into(),
            service_id: Some("orders".into()),
            service_name: Some("orders".into()),
            time_ns: 10,
            trace_id: None,
        });
        let event_message = serde_json::to_value(ServerMessage::Event {
            resume: "12".into(),
            storage_cursor: "11".into(),
            item: &event_item,
            record: event,
        })
        .unwrap();
        assert_eq!(event_message["type"], "event");
        assert_eq!(event_message["record"]["id"], "event-1");
        assert!(event_message.get("items").is_none());

        let snapshot = InfrastructureSnapshot::default();
        let infra =
            serde_json::to_value(ServerMessage::Infrastructure { resume: "13".into(), snapshot: &snapshot }).unwrap();
        assert_eq!(infra["type"], "infrastructure");
        assert_eq!(infra["resume"], "13");
        assert!(infra["snapshot"].is_object());

        let topology = super::super::telemetry::TopologyResponse {
            nodes: vec![],
            edges: vec![],
            from_ns: "1".into(),
            to_ns: "2".into(),
            completeness: super::super::telemetry::Completeness {
                watermark_ns: None,
                processing_caught_up: true,
                unresolved_processing_errors: 0,
                truncated: false,
                sampling: "unknown".into(),
                notes: vec![],
            },
            geometry: karbird_engine_core::SavedTopologyGeometry {
                version: karbird_engine_core::SERVER_LAYOUT_VERSION,
                revision: 1,
                plain: Default::default(),
                infrastructure: None,
                external: vec![],
            },
            updated_at_ns: "14".into(),
            infrastructure_snapshot: None,
        };
        let map = serde_json::to_value(ServerMessage::Topology { resume: "14".into(), snapshot: &topology }).unwrap();
        assert_eq!(map["type"], "topology");
        assert_eq!(map["snapshot"]["updated_at_ns"], "14");
    }

    #[test]
    fn resume_cursor_is_clamped_to_the_live_window() {
        let now = 1_790_000_000_000_000_000;
        assert_eq!(start_cursor(&None, now).unwrap(), now - INITIAL_OVERLAP_NS);
        assert_eq!(start_cursor(&Some((now - 1).to_string()), now).unwrap(), now - 1);
        assert_eq!(start_cursor(&Some("0".into()), now).unwrap(), now - EVENT_LOOKBACK_NS, "no old history");
        assert_eq!(start_cursor(&Some(i64::MIN.to_string()), now).unwrap(), now - EVENT_LOOKBACK_NS);
        assert_eq!(start_cursor(&Some(i64::MAX.to_string()), now).unwrap(), now, "cannot skip ahead");
        for bad in ["", "abc", "1.5", "-"] {
            assert!(matches!(start_cursor(&Some(bad.into()), now), Err(ApiError::BadRequest(_))), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn websocket_origin_is_always_required_for_sessions() {
        let s = crate::state::test_state(true);
        let h = |o: Option<&str>| {
            let mut m = HeaderMap::new();
            if let Some(o) = o {
                m.insert(header::ORIGIN, o.parse().unwrap());
            }
            m
        };
        assert!(require_origin(&s, &h(Some("http://localhost:3000"))).is_ok());
        assert!(require_origin(&s, &h(Some("http://127.0.0.1:3000"))).is_ok());
        // Unlike plain requests, a missing Origin is refused (cross-site WebSocket hijacking).
        for o in [None, Some("http://evil.example"), Some("null"), Some("http://localhost:3000/")] {
            assert!(matches!(require_origin(&s, &h(o)), Err(ApiError::Forbidden("origin not allowed"))), "{o:?}");
        }
    }

    #[test]
    fn credentials_are_rechecked_every_30_seconds() {
        assert_eq!(SESSION_RECHECK, Duration::from_secs(30), "ADR-0011");
    }
}
