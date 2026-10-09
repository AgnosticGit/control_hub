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
use karbird_engine_core::TimelineItem;
use serde::{Deserialize, Serialize};
use tokio::sync::OwnedSemaphorePermit;
use utoipa::IntoParams;

use super::telemetry::{event_item, external_clients_id, log_item, now_ns, parse_ns, span_item};
use crate::auth::{CurrentUser, authorize};
use crate::error::{ApiError, ApiResult};
use crate::live_hub::LiveItem;
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

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LiveParams {
    /// Resume cursor from a previous message (ingestion time, decimal nanoseconds).
    pub resume: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage<'a> {
    // Field order matters: the Worker reads `type` and `resume` from the message head.
    Hello { resume: String, poll_ms: u64 },
    Batch { resume: String, items: &'a [TimelineItem] },
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
    Ok(parse_ns(resume, "resume")?.map_or(now - INITIAL_OVERLAP_NS, |r| r.clamp(floor, now)))
}

#[utoipa::path(get, path = "/api/v1/projects/{project_id}/environments/{environment_id}/live", tag = "telemetry",
    description = "WebSocket upgrade. Server messages: hello, batch, gap, heartbeat, error (JSON text frames).",
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
    let cursor = start_cursor(&q.resume, now_ns())?;
    let permit = state.live_slots.clone().try_acquire_owned().map_err(|_| ApiError::TooManyRequests)?;
    Ok(ws.max_message_size(64 * 1024).on_upgrade(move |socket| run(socket, state, scope, cursor, user, permit)))
}

async fn send(tx: &mut futures::stream::SplitSink<WebSocket, Message>, msg: &ServerMessage<'_>) -> bool {
    match serde_json::to_string(msg) {
        Ok(text) => tx.send(Message::Text(Utf8Bytes::from(text))).await.is_ok(),
        Err(_) => false,
    }
}

async fn run(
    socket: WebSocket,
    state: AppState,
    scope: Scope,
    mut cursor: i64,
    user: CurrentUser,
    _permit: OwnedSemaphorePermit,
) {
    let (mut tx, mut rx) = socket.split();
    let mut last_sent = Instant::now();
    let mut last_session_check = Instant::now();
    let clients_id = external_clients_id(&scope);
    let (replay, mut updates) = match state.live_hub.subscribe(scope.clone(), cursor) {
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
        match tokio::try_join!(
            state.ch.live_spans(&scope, event_from, catchup_after, upto, CATCHUP_LIMIT),
            state.ch.live_logs(&scope, event_from, catchup_after, upto, CATCHUP_LIMIT),
            state.ch.live_events(&scope, event_from, catchup_after, upto, CATCHUP_LIMIT),
        ) {
            Ok((spans, logs, events)) => {
                let truncated = spans.len() >= CATCHUP_LIMIT as usize
                    || logs.len() >= CATCHUP_LIMIT as usize
                    || events.len() >= CATCHUP_LIMIT as usize;
                let mut items = Vec::with_capacity(spans.len() + logs.len() + events.len());
                items.extend(
                    spans
                        .into_iter()
                        .map(|row| LiveItem { cursor_ns: row.ingested_at, item: span_item(row, &clients_id) }),
                );
                items.extend(logs.into_iter().map(|row| LiveItem { cursor_ns: row.ingested_at, item: log_item(row) }));
                items.extend(
                    events.into_iter().map(|row| LiveItem { cursor_ns: row.ingested_at, item: event_item(row) }),
                );
                items.sort_by_key(|item| item.cursor_ns);
                let timeline: Vec<_> = items.iter().map(|item| item.item.clone()).collect();
                let ok = if truncated {
                    send(
                        &mut tx,
                        &ServerMessage::Gap { resume: upto.to_string(), from_ingested_ns: catchup_after.to_string() },
                    )
                    .await
                } else if timeline.is_empty() {
                    true
                } else {
                    send(&mut tx, &ServerMessage::Batch { resume: upto.to_string(), items: &timeline }).await
                };
                if !ok {
                    return;
                }
                cursor = cursor.max(upto);
            }
            Err(error) => {
                tracing::warn!(%error, "live catch-up query failed");
                let _ = send(&mut tx, &ServerMessage::Error { message: "live catch-up unavailable" }).await;
                return;
            }
        }
    }
    let mut pending = replay.into_iter().filter(|item| item.cursor_ns > cursor).collect::<Vec<_>>();
    pending.sort_by_key(|item| item.cursor_ns);
    if !pending.is_empty() {
        cursor = pending.iter().map(|item| item.cursor_ns).max().unwrap_or(cursor);
        let timeline: Vec<_> = pending.into_iter().map(|item| item.item).collect();
        if !send(&mut tx, &ServerMessage::Batch { resume: cursor.to_string(), items: &timeline }).await {
            return;
        }
        last_sent = Instant::now();
    }
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            incoming = rx.next() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                // Client data is not used; pings are answered by the WebSocket layer.
                Some(Ok(_)) => {}
            },
            update = updates.recv() => {
                let Some(mut batch) = update else {
                    let _ = send(&mut tx, &ServerMessage::Gap { resume: cursor.to_string(), from_ingested_ns: cursor.to_string() }).await;
                    break;
                };
                batch.retain(|item| item.cursor_ns > cursor);
                if batch.is_empty() { continue; }
                let mut items = batch;
                cursor = items.iter().map(|item| item.cursor_ns).max().unwrap_or(cursor);
                while items.len() < 128 {
                    match updates.try_recv() {
                        Ok(mut next) => {
                            next.retain(|item| item.cursor_ns > cursor);
                            if !next.is_empty() {
                                cursor = next.iter().map(|item| item.cursor_ns).max().unwrap_or(cursor);
                                items.extend(next);
                            }
                        }
                        Err(_) => break,
                    }
                }
                let timeline: Vec<_> = items.into_iter().map(|item| item.item).collect();
                if !send(&mut tx, &ServerMessage::Batch { resume: cursor.to_string(), items: &timeline }).await { break; }
                last_sent = Instant::now();
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
    fn batch_message_head_is_type_then_resume_and_engine_accepts_it() {
        let items = vec![TimelineItem::Log(karbird_engine_core::LogItem {
            id: "l1".into(),
            service_id: "svc_a".into(),
            service_name: "a".into(),
            time_ns: 1_790_000_000_000_000_000,
            severity: 9,
            trace_id: None,
        })];
        let text = serde_json::to_string(&ServerMessage::Batch { resume: "42".into(), items: &items }).unwrap();
        assert!(text.starts_with(r#"{"type":"batch","resume":"42","items":["#), "{text}");
        let mut e = karbird_engine_core::Engine::new(Default::default());
        e.apply_batch_json(&text).unwrap();
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
