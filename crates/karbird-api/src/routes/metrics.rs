//! Internal Prometheus metrics for the API's Kafka live reader.

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;

use crate::state::AppState;

/// This route is available on the private API listener only. Public Caddy routes do not proxy it.
pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        state.live_metrics.render_prometheus(&state.live_hub),
    )
}
