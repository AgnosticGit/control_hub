//! OTLP/HTTP (binary Protobuf) and the custom events API.
//!
//! Supported: `POST /v1/traces`, `POST /v1/logs` with
//! `Content-Type: application/x-protobuf`, optional `Content-Encoding: gzip`.
//! Not supported (explicit errors, never silent success): OTLP/HTTP JSON
//! encoding (415), `/v1/metrics` (501).

use std::io::Read;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use controlhub_domain::Signal;
use controlhub_protocol::otlp::collector::logs::v1::{
    ExportLogsPartialSuccess, ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use controlhub_protocol::otlp::collector::trace::v1::{
    ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use controlhub_protocol::v1::ingest_envelope::Payload;
use prost::Message;
use tower::ServiceBuilder;
use tower_http::timeout::TimeoutLayer;

use crate::auth::AuthError;
use crate::broker::PublishError;
use crate::events;
use crate::metrics::Metrics;
use crate::pipeline::{AcceptError, AppState};
use crate::sanitize::{sanitize_logs, sanitize_traces};

pub const MAX_COMPRESSED_BODY: usize = 4 * 1024 * 1024;
pub const MAX_DECOMPRESSED_BODY: usize = 16 * 1024 * 1024;

/// `google.rpc.Status`, required by OTLP/HTTP as the error body.
#[derive(Clone, PartialEq, prost::Message)]
pub struct RpcStatus {
    #[prost(int32, tag = "1")]
    pub code: i32,
    #[prost(string, tag = "2")]
    pub message: String,
}

pub fn router(state: AppState, max_concurrency: usize) -> Router {
    Router::new()
        .route("/v1/traces", post(export_traces))
        .route("/v1/logs", post(export_logs))
        .route("/v1/metrics", post(export_metrics_unsupported))
        .route("/api/v1/events/batch", post(events_batch))
        .route("/health", get(|| async { "ok" }))
        .layer(
            ServiceBuilder::new()
                .layer(DefaultBodyLimit::max(MAX_COMPRESSED_BODY))
                .layer(TimeoutLayer::with_status_code(StatusCode::SERVICE_UNAVAILABLE, Duration::from_secs(20)))
                .layer(tower::limit::ConcurrencyLimitLayer::new(max_concurrency)),
        )
        .with_state(state)
}

fn protobuf_error(status: StatusCode, grpc_code: i32, msg: impl Into<String>, retry_after: Option<u64>) -> Response {
    let body = RpcStatus { code: grpc_code, message: msg.into() }.encode_to_vec();
    let mut resp = (status, body).into_response();
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-protobuf"));
    if let Some(s) = retry_after
        && let Ok(v) = HeaderValue::from_str(&s.to_string())
    {
        resp.headers_mut().insert(header::RETRY_AFTER, v);
    }
    resp
}

fn accept_error_response(e: AcceptError) -> Response {
    // gRPC codes: 16 unauthenticated, 7 permission denied, 3 invalid argument, 14 unavailable, 8 resource exhausted
    match e {
        AcceptError::Auth(AuthError::Missing | AuthError::Invalid) => {
            protobuf_error(StatusCode::UNAUTHORIZED, 16, "invalid or missing ingestion token", None)
        }
        AcceptError::Auth(AuthError::SignalNotAllowed(s)) => {
            protobuf_error(StatusCode::FORBIDDEN, 7, format!("token not allowed to send {s}"), None)
        }
        AcceptError::Auth(AuthError::Unavailable) => {
            protobuf_error(StatusCode::SERVICE_UNAVAILABLE, 14, "credential store unavailable", Some(2))
        }
        AcceptError::Publish(PublishError::Overloaded) => {
            protobuf_error(StatusCode::TOO_MANY_REQUESTS, 8, "ingest queue full, retry later", Some(1))
        }
        AcceptError::Publish(PublishError::Timeout | PublishError::Unavailable(_)) => protobuf_error(
            StatusCode::SERVICE_UNAVAILABLE,
            14,
            "broker did not acknowledge the write; data NOT accepted, retry",
            Some(2),
        ),
    }
}

/// Validate content type and decode (optionally gzip) body with a decompressed-size limit.
#[allow(clippy::result_large_err)]
fn decode_body(headers: &HeaderMap, body: &Bytes) -> Result<Vec<u8>, Response> {
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    let mime = ct.split(';').next().unwrap_or("").trim();
    if mime == "application/json" {
        return Err(protobuf_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            3,
            "OTLP/HTTP JSON encoding is not supported yet; use application/x-protobuf",
            None,
        ));
    }
    if mime != "application/x-protobuf" {
        return Err(protobuf_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, 3, "expected application/x-protobuf", None));
    }
    let encoding = headers.get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()).unwrap_or("identity");
    match encoding {
        "identity" | "" => Ok(body.to_vec()),
        "gzip" => {
            let mut out = Vec::new();
            let mut dec = flate2::read::GzDecoder::new(body.as_ref()).take(MAX_DECOMPRESSED_BODY as u64 + 1);
            dec.read_to_end(&mut out)
                .map_err(|_| protobuf_error(StatusCode::BAD_REQUEST, 3, "invalid gzip body", None))?;
            if out.len() > MAX_DECOMPRESSED_BODY {
                return Err(protobuf_error(StatusCode::PAYLOAD_TOO_LARGE, 3, "decompressed body too large", None));
            }
            Ok(out)
        }
        other => Err(protobuf_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            3,
            format!("unsupported content-encoding {other}"),
            None,
        )),
    }
}

fn auth_header(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok())
}

fn protobuf_ok(bytes: Vec<u8>) -> Response {
    let mut resp = (StatusCode::OK, bytes).into_response();
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/x-protobuf"));
    resp
}

async fn export_traces(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let cred = match st.authenticate(auth_header(&headers), Signal::Traces).await {
        Ok(c) => c,
        Err(e) => return accept_error_response(e),
    };
    let raw = match decode_body(&headers, &body) {
        Ok(r) => r,
        Err(r) => {
            Metrics::inc(&st.metrics.requests_rejected_invalid, 1);
            return r;
        }
    };
    let mut req = match ExportTraceServiceRequest::decode(raw.as_slice()) {
        Ok(r) => r,
        Err(_) => {
            Metrics::inc(&st.metrics.requests_rejected_invalid, 1);
            return protobuf_error(StatusCode::BAD_REQUEST, 3, "malformed ExportTraceServiceRequest", None);
        }
    };
    let stats = sanitize_traces(&mut req);
    Metrics::inc(&st.metrics.attributes_redacted, stats.redacted);
    Metrics::inc(&st.metrics.spans_rejected, stats.rejected);
    if stats.accepted > 0 {
        if let Err(e) = st.publish(&cred, Payload::OtlpTraces(req.encode_to_vec())).await {
            return accept_error_response(e);
        }
        Metrics::inc(&st.metrics.spans_accepted, stats.accepted);
    }
    let resp = ExportTraceServiceResponse {
        partial_success: (stats.rejected > 0).then(|| ExportTracePartialSuccess {
            rejected_spans: stats.rejected as i64,
            error_message: "spans with invalid ids/timestamps or over the per-request limit were rejected".into(),
        }),
    };
    protobuf_ok(resp.encode_to_vec())
}

async fn export_logs(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let cred = match st.authenticate(auth_header(&headers), Signal::Logs).await {
        Ok(c) => c,
        Err(e) => return accept_error_response(e),
    };
    let raw = match decode_body(&headers, &body) {
        Ok(r) => r,
        Err(r) => {
            Metrics::inc(&st.metrics.requests_rejected_invalid, 1);
            return r;
        }
    };
    let mut req = match ExportLogsServiceRequest::decode(raw.as_slice()) {
        Ok(r) => r,
        Err(_) => {
            Metrics::inc(&st.metrics.requests_rejected_invalid, 1);
            return protobuf_error(StatusCode::BAD_REQUEST, 3, "malformed ExportLogsServiceRequest", None);
        }
    };
    let stats = sanitize_logs(&mut req);
    Metrics::inc(&st.metrics.attributes_redacted, stats.redacted);
    Metrics::inc(&st.metrics.logs_rejected, stats.rejected);
    if stats.accepted > 0 {
        if let Err(e) = st.publish(&cred, Payload::OtlpLogs(req.encode_to_vec())).await {
            return accept_error_response(e);
        }
        Metrics::inc(&st.metrics.logs_accepted, stats.accepted);
    }
    let resp = ExportLogsServiceResponse {
        partial_success: (stats.rejected > 0).then(|| ExportLogsPartialSuccess {
            rejected_log_records: stats.rejected as i64,
            error_message: "log records with invalid ids/timestamps or over the per-request limit were rejected".into(),
        }),
    };
    protobuf_ok(resp.encode_to_vec())
}

async fn export_metrics_unsupported() -> Response {
    protobuf_error(
        StatusCode::NOT_IMPLEMENTED,
        12,
        "OTLP metrics ingestion is not implemented yet in ControlHub; data was NOT accepted",
        None,
    )
}

async fn events_batch(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let json_err = |status: StatusCode, msg: &str| (status, Json(serde_json::json!({ "error": msg }))).into_response();
    let cred = match st.authenticate(auth_header(&headers), Signal::Events).await {
        Ok(c) => c,
        Err(AcceptError::Auth(AuthError::Unavailable)) => {
            return json_err(StatusCode::SERVICE_UNAVAILABLE, "credential store unavailable");
        }
        Err(AcceptError::Auth(AuthError::SignalNotAllowed(_))) => {
            return json_err(StatusCode::FORBIDDEN, "token not allowed to send events");
        }
        Err(_) => return json_err(StatusCode::UNAUTHORIZED, "invalid or missing ingestion token"),
    };
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return json_err(StatusCode::UNSUPPORTED_MEDIA_TYPE, "expected application/json");
    }
    let (batch, rejected) = match events::parse_batch(&body) {
        Ok(v) => v,
        Err(msg) => {
            Metrics::inc(&st.metrics.requests_rejected_invalid, 1);
            return json_err(StatusCode::BAD_REQUEST, &msg);
        }
    };
    Metrics::inc(&st.metrics.events_rejected, rejected.len() as u64);
    let accepted = batch.events.len();
    if accepted == 0 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "accepted": 0, "rejected": rejected })))
            .into_response();
    }
    match st.publish(&cred, Payload::CustomEvents(batch)).await {
        Ok(a) => {
            Metrics::inc(&st.metrics.events_accepted, accepted as u64);
            Json(serde_json::json!({
                "batch_id": a.batch_id,
                "accepted": accepted,
                "rejected": rejected,
                "durability": "broker_acknowledged",
                "note": "accepted events become queryable after processing; this is not a storage visibility guarantee"
            }))
            .into_response()
        }
        Err(AcceptError::Publish(PublishError::Overloaded)) => {
            let mut r = json_err(StatusCode::TOO_MANY_REQUESTS, "ingest queue full, retry later");
            r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            r
        }
        Err(_) => {
            let mut r =
                json_err(StatusCode::SERVICE_UNAVAILABLE, "broker did not acknowledge; events NOT accepted, retry");
            r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
            r
        }
    }
}
