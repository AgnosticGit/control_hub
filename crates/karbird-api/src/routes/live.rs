//! WebSocket live channel for one environment.
//!
//! One bounded catch-up query on connect recovers gaps from disconnects; steady-state updates
//! arrive from the API replica's Kafka consumer through the scoped in-process live hub.
//!
//! Only items of the authorized environment are sent; no tenant-wide stream exists.

use std::time::{Duration, Instant};

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use karbird_domain::Scope;
use karbird_engine_core::{InfrastructureSnapshot, TimelineItem};
use serde::{Deserialize, Serialize};
use tokio::sync::OwnedSemaphorePermit;
use utoipa::IntoParams;

use super::telemetry::{
    event_dto, event_item, external_clients_id, log_dto, log_item, now_ns, parse_ns, span_dto, span_item,
};
use crate::auth::{CurrentUser, authorize};
use crate::error::{ApiError, ApiResult};
use crate::live_hub::{LiveItem, LiveSignal, LiveUpdate};
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
    Hello { resume: String, poll_ms: u64 },
    Span { resume: String, storage_cursor: String, item: &'a TimelineItem, record: &'a super::telemetry::SpanDto },
    Log { resume: String, storage_cursor: String, item: &'a TimelineItem, record: &'a super::telemetry::LogDto },
    Event { resume: String, storage_cursor: String, item: &'a TimelineItem, record: &'a super::telemetry::EventDto },
    Action { resume: String, plan: serde_json::Value, snapshot: bool },
    Topology { resume: String, snapshot: &'a super::telemetry::TopologyResponse },
    Infrastructure { resume: String, snapshot: &'a InfrastructureSnapshot },
    Gap { resume: String, from_ingested_ns: String },
    Heartbeat { resume: String },
    Error { message: &'a str },
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

async fn run(socket: WebSocket, state: AppState, scope: Scope, mut cursor: i64, session: LiveSession) {
    let LiveSession { user, _permit, infrastructure, actions_enabled } = session;
    let (mut tx, mut rx) = socket.split();
    let mut last_sent = Instant::now();
    let mut last_session_check = Instant::now();
    let clients_id = external_clients_id(&scope);
    let (replay, mut updates) = match state.live_hub.subscribe_with_actions(scope.clone(), cursor, actions_enabled) {
        Ok(value) => value,
        Err(error) => {
            let _ = send(&mut tx, &ServerMessage::Error { message: "live channel unavailable" }).await;
            tracing::warn!(%error, "live subscription rejected");
            return;
        }
    };
    if !send(&mut tx, &ServerMessage::Hello { resume: cursor.to_string(), poll_ms: 0 }).await {
        return;
    }
    // One bounded catch-up per socket handles API restarts and reconnects. The subscription is
    // installed first, so Kafka items arriving during these queries remain queued for delivery.
    let upto = now_ns().saturating_sub(VISIBILITY_MARGIN_NS);
    let catchup_after = cursor.saturating_sub(CATCHUP_OVERLAP_NS);
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
                // Client data is not used; pings are answered by the WebSocket layer.
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

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
