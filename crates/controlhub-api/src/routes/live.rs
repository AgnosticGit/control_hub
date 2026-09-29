//! WebSocket live channel for one environment.
//!
//! The server polls ClickHouse by *ingestion* time `(cursor, now - margin]` and pushes new
//! timeline items, so late-arriving telemetry is delivered as well. The margin gives
//! concurrent processor inserts time to become visible. Every message carries a `resume`
//! cursor; a reconnect with `?resume=` continues without gaps (duplicates are harmless, the
//! engine deduplicates by item ID). When a poll hits the size limit the server sends `gap`
//! instead of silently dropping, and the client reloads that range over HTTP.
//!
//! Only items of the authorized environment are sent; no tenant-wide stream exists.

use std::time::{Duration, Instant};

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use controlhub_domain::Scope;
use controlhub_engine_core::TimelineItem;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::OwnedSemaphorePermit;
use utoipa::IntoParams;

use super::telemetry::{event_item, log_item, now_ns, parse_ns, span_item};
use crate::auth::{CurrentUser, Role, authorize};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const POLL: Duration = Duration::from_secs(1);
const HEARTBEAT: Duration = Duration::from_secs(15);
const SESSION_RECHECK: Duration = Duration::from_secs(30);
/// Rows ingested within the last `VISIBILITY_MARGIN_NS` are not read yet.
const VISIBILITY_MARGIN_NS: i64 = 3_000_000_000;
/// A fresh subscription starts this far back (overlaps the client's initial HTTP load).
const INITIAL_OVERLAP_NS: i64 = 30_000_000_000;
/// Only items whose event time is within this window are streamed (matches the live window).
const EVENT_LOOKBACK_NS: i64 = 15 * 60_000_000_000;
const MAX_ROWS_PER_KIND: u32 = 5_000;
const MAX_QUERY_FAILURES: u32 = 5;

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
    let (scope, _) = authorize(&state, &user, &project_id, &environment_id, Role::Viewer).await?;
    let now = now_ns();
    let floor = now - EVENT_LOOKBACK_NS;
    let cursor = parse_ns(&q.resume, "resume")?.map_or(now - INITIAL_OVERLAP_NS, |r| r.clamp(floor, now));
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
    let mut tick = tokio::time::interval(POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_sent = Instant::now();
    let mut last_session_check = Instant::now();
    let mut failures = 0u32;

    if !send(&mut tx, &ServerMessage::Hello { resume: cursor.to_string(), poll_ms: POLL.as_millis() as u64 }).await {
        return;
    }
    loop {
        tokio::select! {
            incoming = rx.next() => match incoming {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                // Client data is not used; pings are answered by the WebSocket layer.
                Some(Ok(_)) => {}
            },
            _ = tick.tick() => {
                if last_session_check.elapsed() >= SESSION_RECHECK {
                    last_session_check = Instant::now();
                    if !user.still_active(&state).await.unwrap_or(false) {
                        let _ = send(&mut tx, &ServerMessage::Error { message: "authorization ended" }).await;
                        break;
                    }
                }
                let upto = now_ns() - VISIBILITY_MARGIN_NS;
                if upto <= cursor {
                    continue;
                }
                let event_from = upto - EVENT_LOOKBACK_NS;
                let fetched = tokio::try_join!(
                    state.ch.live_spans(&scope, event_from, cursor, upto, MAX_ROWS_PER_KIND),
                    state.ch.live_logs(&scope, event_from, cursor, upto, MAX_ROWS_PER_KIND),
                    state.ch.live_events(&scope, event_from, cursor, upto, MAX_ROWS_PER_KIND),
                );
                let (spans, logs, events) = match fetched {
                    Ok(v) => { failures = 0; v }
                    Err(e) => {
                        failures += 1;
                        tracing::warn!(error = %e, failures, "live query failed");
                        if failures >= MAX_QUERY_FAILURES {
                            let _ = send(&mut tx, &ServerMessage::Error { message: "live query failing" }).await;
                            break;
                        }
                        continue;
                    }
                };
                let limit = MAX_ROWS_PER_KIND as usize;
                let from = cursor;
                cursor = upto;
                let ok = if spans.len() >= limit || logs.len() >= limit || events.len() >= limit {
                    send(&mut tx, &ServerMessage::Gap { resume: cursor.to_string(), from_ingested_ns: from.to_string() }).await
                } else if spans.is_empty() && logs.is_empty() && events.is_empty() {
                    if last_sent.elapsed() < HEARTBEAT {
                        continue;
                    }
                    send(&mut tx, &ServerMessage::Heartbeat { resume: cursor.to_string() }).await
                } else {
                    let mut items = Vec::with_capacity(spans.len() + logs.len() + events.len());
                    items.extend(spans.into_iter().map(span_item));
                    items.extend(logs.into_iter().map(log_item));
                    items.extend(events.into_iter().map(event_item));
                    send(&mut tx, &ServerMessage::Batch { resume: cursor.to_string(), items: &items }).await
                };
                if !ok {
                    break;
                }
                last_sent = Instant::now();
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
        let items = vec![TimelineItem::Log(controlhub_engine_core::LogItem {
            id: "l1".into(),
            service_id: "svc_a".into(),
            service_name: "a".into(),
            time_ns: 1_790_000_000_000_000_000,
            severity: 9,
            trace_id: None,
        })];
        let text = serde_json::to_string(&ServerMessage::Batch { resume: "42".into(), items: &items }).unwrap();
        assert!(text.starts_with(r#"{"type":"batch","resume":"42","items":["#), "{text}");
        let mut e = controlhub_engine_core::Engine::new(Default::default());
        e.apply_batch_json(&text).unwrap();
        assert_eq!(e.item_count(), 1);
    }
}
