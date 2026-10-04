//! OTLP/HTTP (binary Protobuf) and the custom events API.
//!
//! Supported: `POST /v1/traces`, `POST /v1/logs`, `POST /v1/metrics` (gauges and
//! sums, ADR-0018) with `Content-Type: application/x-protobuf`, optional
//! `Content-Encoding: gzip`. Not supported (explicit errors, never silent
//! success): OTLP/HTTP JSON encoding (415).

use std::io::Read;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use karbird_domain::Signal;
use karbird_protocol::otlp::collector::logs::v1::{
    ExportLogsPartialSuccess, ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use karbird_protocol::otlp::collector::metrics::v1::{
    ExportMetricsPartialSuccess, ExportMetricsServiceRequest, ExportMetricsServiceResponse,
};
use karbird_protocol::otlp::collector::trace::v1::{
    ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use karbird_protocol::v1::ingest_envelope::Payload;
use prost::Message;
use tower::ServiceBuilder;
use tower_http::timeout::TimeoutLayer;

use crate::auth::AuthError;
use crate::broker::PublishError;
use crate::events;
use crate::metrics::Metrics;
use crate::pipeline::{AcceptError, AppState};
use crate::sanitize::{MetricStats, sanitize_logs, sanitize_traces};

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
        .route("/v1/metrics", post(export_metrics))
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
    // Media types are case-insensitive (RFC 9110 §8.3.1).
    let mime = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
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
    Metrics::inc(&st.metrics.attributes_truncated, stats.truncated_attrs);
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
    Metrics::inc(&st.metrics.attributes_truncated, stats.truncated_attrs);
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

async fn export_metrics(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let cred = match st.authenticate(auth_header(&headers), Signal::Metrics).await {
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
    let mut req = match ExportMetricsServiceRequest::decode(raw.as_slice()) {
        Ok(r) => r,
        Err(_) => {
            Metrics::inc(&st.metrics.requests_rejected_invalid, 1);
            return protobuf_error(StatusCode::BAD_REQUEST, 3, "malformed ExportMetricsServiceRequest", None);
        }
    };
    let stats = st.sanitize_metrics(&cred, &mut req);
    if stats.accepted > 0 {
        if let Err(e) = st.publish(&cred, Payload::OtlpMetrics(req.encode_to_vec())).await {
            return accept_error_response(e);
        }
        Metrics::inc(&st.metrics.metric_points_accepted, stats.accepted);
    }
    let resp = ExportMetricsServiceResponse { partial_success: metrics_partial_success(&stats) };
    protobuf_ok(resp.encode_to_vec())
}

pub(crate) fn metrics_partial_success(stats: &MetricStats) -> Option<ExportMetricsPartialSuccess> {
    (stats.refused() > 0).then(|| ExportMetricsPartialSuccess {
        rejected_data_points: stats.refused() as i64,
        error_message: if stats.over_limit > 0 {
            format!(
                "{} points of new series over the per-environment series limit and {} invalid or unsupported points \
                 (only gauges and sums are accepted) were rejected",
                stats.over_limit, stats.rejected
            )
        } else {
            "invalid or unsupported points (only gauges and sums are accepted) or over the per-request limit were rejected"
                .into()
        },
    })
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
        .is_some_and(|v| v.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"));
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::broker::fake::Mode;
    use crate::testing::{Harness, TOKEN, TRACES_ONLY, get, harness};
    use axum::body::Body;
    use axum::http::Request;
    use karbird_protocol::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
    use karbird_protocol::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use karbird_protocol::otlp::metrics::v1::{
        Histogram, HistogramDataPoint, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum, metric::Data,
        number_data_point,
    };
    use karbird_protocol::otlp::resource::v1::Resource;
    use karbird_protocol::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use serde_json::{Value as Json, json};
    use tower::ServiceExt;

    const PB: &str = "application/x-protobuf";

    fn kv(k: &str, v: &str) -> KeyValue {
        KeyValue {
            key: k.into(),
            value: Some(AnyValue { value: Some(Value::StringValue(v.into())) }),
            ..Default::default()
        }
    }

    fn good_span() -> Span {
        Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            name: "GET /orders".into(),
            start_time_unix_nano: 10,
            end_time_unix_nano: 20,
            attributes: vec![kv("http.request.header.authorization", "Bearer leaked-token"), kv("http.route", "/o")],
            ..Default::default()
        }
    }

    fn bad_span() -> Span {
        Span { trace_id: vec![0; 16], ..good_span() }
    }

    fn traces(spans: Vec<Span>) -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![kv("service.name", "orders"), kv("tenant_id", "someone-else")],
                    ..Default::default()
                }),
                scope_spans: vec![ScopeSpans { spans, ..Default::default() }],
                ..Default::default()
            }],
        }
    }

    fn logs(records: Vec<LogRecord>) -> ExportLogsServiceRequest {
        ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs { log_records: records, ..Default::default() }],
                ..Default::default()
            }],
        }
    }

    fn good_log() -> LogRecord {
        LogRecord {
            time_unix_nano: 5,
            body: Some(AnyValue { value: Some(Value::StringValue("login with Bearer abc123".into())) }),
            attributes: vec![kv("user.password", "hunter2")],
            ..Default::default()
        }
    }

    fn post(path: &str, token: Option<&str>, content_type: Option<&str>, body: impl Into<Body>) -> Request<Body> {
        let mut b = Request::post(path);
        if let Some(t) = token {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        if let Some(ct) = content_type {
            b = b.header(header::CONTENT_TYPE, ct);
        }
        b.body(body.into()).unwrap()
    }

    async fn send(h: &Harness, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let resp = router(h.state.clone(), 16).oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec();
        (status, headers, body)
    }

    fn rpc_status(headers: &HeaderMap, body: &[u8]) -> RpcStatus {
        assert_eq!(headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()), Some(PB));
        RpcStatus::decode(body).unwrap()
    }

    fn retry_after(headers: &HeaderMap) -> Option<&str> {
        headers.get(header::RETRY_AFTER).and_then(|v| v.to_str().ok())
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(bytes).unwrap();
        enc.finish().unwrap()
    }

    fn published_traces(h: &Harness) -> Vec<ExportTraceServiceRequest> {
        h.envelopes()
            .into_iter()
            .map(|e| match e.payload {
                Some(Payload::OtlpTraces(b)) => ExportTraceServiceRequest::decode(b.as_slice()).unwrap(),
                other => panic!("unexpected payload {other:?}"),
            })
            .collect()
    }

    // ---- plumbing ----

    #[tokio::test]
    async fn health_is_public() {
        let h = harness();
        let (status, _, body) = send(&h, Request::get("/health").body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"ok");
    }

    // ---- metrics ----

    fn cpu_metrics(host: &str, cpus: usize) -> ExportMetricsServiceRequest {
        let points = (0..cpus)
            .map(|i| NumberDataPoint {
                attributes: vec![kv("cpu", &format!("cpu{i}")), kv("state", "user")],
                time_unix_nano: 10,
                value: Some(number_data_point::Value::AsDouble(1.5)),
                ..Default::default()
            })
            .collect();
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![
                        kv("host.name", host),
                        kv("tenant_id", "someone-else"),
                        kv("auth_token", "s3cr3t"),
                    ],
                    ..Default::default()
                }),
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![Metric {
                        name: "system.cpu.time".into(),
                        unit: "s".into(),
                        data: Some(Data::Sum(Sum {
                            data_points: points,
                            aggregation_temporality: 2,
                            is_monotonic: true,
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    fn published_metrics(h: &Harness) -> Vec<ExportMetricsServiceRequest> {
        h.envelopes()
            .into_iter()
            .map(|e| match e.payload {
                Some(Payload::OtlpMetrics(b)) => ExportMetricsServiceRequest::decode(b.as_slice()).unwrap(),
                other => panic!("unexpected payload {other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn metrics_are_accepted_redacted_and_scoped_by_the_token() {
        let h = harness();
        let body = cpu_metrics("vps-app", 2).encode_to_vec();
        let (status, headers, resp) = send(&h, post("/v1/metrics", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()), Some(PB));
        assert!(ExportMetricsServiceResponse::decode(resp.as_slice()).unwrap().partial_success.is_none());
        assert_eq!(h.envelopes()[0].scope.as_ref().unwrap().tenant_id, "t1");
        let dump = format!("{:?}", published_metrics(&h));
        assert!(dump.contains("vps-app") && !dump.contains("s3cr3t"), "{dump}");
        assert_eq!(get(&h.metrics().metric_points_accepted), 2);
        assert_eq!(get(&h.metrics().attributes_redacted), 1);
    }

    #[tokio::test]
    async fn metrics_need_the_metrics_signal_and_a_token() {
        let h = harness();
        let body = cpu_metrics("h", 1).encode_to_vec();
        let (status, headers, resp) = send(&h, post("/v1/metrics", None, Some(PB), body.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(rpc_status(&headers, &resp).code, 16);
        let (status, headers, resp) = send(&h, post("/v1/metrics", Some(TRACES_ONLY), Some(PB), body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(rpc_status(&headers, &resp).message.contains("metrics"));
        assert!(h.broker.records().is_empty());
    }

    #[tokio::test]
    async fn histograms_are_rejected_with_partial_success() {
        let h = harness();
        let mut req = cpu_metrics("h", 1);
        req.resource_metrics[0].scope_metrics[0].metrics.push(Metric {
            name: "http.server.duration".into(),
            data: Some(Data::Histogram(Histogram {
                data_points: vec![HistogramDataPoint::default(); 3],
                aggregation_temporality: 2,
            })),
            ..Default::default()
        });
        let (status, _, resp) = send(&h, post("/v1/metrics", Some(TOKEN), Some(PB), req.encode_to_vec())).await;
        assert_eq!(status, StatusCode::OK);
        let ps = ExportMetricsServiceResponse::decode(resp.as_slice()).unwrap().partial_success.unwrap();
        assert_eq!(ps.rejected_data_points, 3);
        assert!(ps.error_message.contains("gauges and sums"));
        assert_eq!(get(&h.metrics().metric_points_rejected), 3);
        let published = published_metrics(&h);
        assert_eq!(published[0].resource_metrics[0].scope_metrics[0].metrics.len(), 1);
    }

    #[tokio::test]
    async fn series_over_the_environment_limit_are_refused_and_counted() {
        let h = harness();
        let body = cpu_metrics("h", crate::testing::TEST_MAX_SERIES + 2).encode_to_vec();
        let (status, _, resp) = send(&h, post("/v1/metrics", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::OK);
        let ps = ExportMetricsServiceResponse::decode(resp.as_slice()).unwrap().partial_success.unwrap();
        assert_eq!(ps.rejected_data_points, 2);
        assert!(ps.error_message.contains("series limit"));
        assert_eq!(get(&h.metrics().metric_series_rejected), 2);
        assert_eq!(get(&h.metrics().metric_points_accepted), crate::testing::TEST_MAX_SERIES as u64);
        // The same series keep flowing on the next export.
        let body = cpu_metrics("h", crate::testing::TEST_MAX_SERIES).encode_to_vec();
        let (_, _, resp) = send(&h, post("/v1/metrics", Some(TOKEN), Some(PB), body)).await;
        assert!(ExportMetricsServiceResponse::decode(resp.as_slice()).unwrap().partial_success.is_none());
    }

    #[tokio::test]
    async fn all_rejected_metrics_are_not_published() {
        let h = harness();
        let mut req = cpu_metrics("h", 1);
        req.resource_metrics[0].scope_metrics[0].metrics[0].name = "bad name".into();
        let (status, _, resp) = send(&h, post("/v1/metrics", Some(TOKEN), Some(PB), req.encode_to_vec())).await;
        assert_eq!(status, StatusCode::OK);
        let ps = ExportMetricsServiceResponse::decode(resp.as_slice()).unwrap().partial_success.unwrap();
        assert_eq!(ps.rejected_data_points, 1);
        assert!(h.broker.records().is_empty());
    }

    #[tokio::test]
    async fn metrics_broker_failure_is_503_never_200() {
        let h = harness();
        h.broker.set_mode(Mode::Fail(PublishError::Unavailable("down".into())));
        let body = cpu_metrics("h", 1).encode_to_vec();
        let (status, headers, resp) = send(&h, post("/v1/metrics", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(rpc_status(&headers, &resp).message.contains("NOT accepted"));
        assert_eq!(get(&h.metrics().metric_points_accepted), 0);
    }

    #[tokio::test]
    async fn unknown_routes_and_methods_are_rejected() {
        let h = harness();
        let (status, _, _) = send(&h, post("/v1/profiles", Some(TOKEN), Some(PB), vec![])).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = send(&h, Request::get("/v1/traces").body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    // ---- authentication ----

    #[tokio::test]
    async fn missing_token_is_401_and_nothing_is_published() {
        let h = harness();
        let body = traces(vec![good_span()]).encode_to_vec();
        let (status, headers, resp) = send(&h, post("/v1/traces", None, Some(PB), body)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(rpc_status(&headers, &resp).code, 16);
        assert_eq!(get(&h.metrics().requests_rejected_auth), 1);
        assert!(h.broker.records().is_empty());
    }

    #[tokio::test]
    async fn invalid_token_is_401() {
        let h = harness();
        let (status, headers, resp) = send(&h, post("/v1/logs", Some("nope"), Some(PB), vec![])).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(rpc_status(&headers, &resp).code, 16);
        let req = Request::post("/v1/traces").header(header::AUTHORIZATION, "Basic dXNlcjpwdw==").body(Body::empty());
        let (status, _, _) = send(&h, req.unwrap()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(get(&h.metrics().requests_rejected_auth), 2);
    }

    #[tokio::test]
    async fn signal_not_granted_is_403() {
        let h = harness();
        let body = logs(vec![good_log()]).encode_to_vec();
        let (status, headers, resp) = send(&h, post("/v1/logs", Some(TRACES_ONLY), Some(PB), body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let s = rpc_status(&headers, &resp);
        assert_eq!(s.code, 7);
        assert!(s.message.contains("logs"));
        assert!(h.broker.records().is_empty());
    }

    #[tokio::test]
    async fn credential_store_outage_is_503_with_retry_after() {
        let h = harness();
        h.store.lock().unwrap().unavailable = true;
        let (status, headers, resp) = send(&h, post("/v1/traces", Some(TOKEN), Some(PB), vec![])).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(rpc_status(&headers, &resp).code, 14);
        assert_eq!(retry_after(&headers), Some("2"));
    }

    #[tokio::test]
    async fn auth_is_checked_before_the_body_is_parsed() {
        let h = harness();
        let (status, _, _) = send(&h, post("/v1/traces", None, Some("text/plain"), "garbage")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(get(&h.metrics().requests_rejected_invalid), 0);
    }

    // ---- content negotiation and decoding ----

    #[tokio::test]
    async fn json_encoding_is_415_not_silently_accepted() {
        let h = harness();
        let (status, headers, resp) = send(&h, post("/v1/traces", Some(TOKEN), Some("application/json"), "{}")).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let s = rpc_status(&headers, &resp);
        assert_eq!(s.code, 3);
        assert!(s.message.contains("JSON"));
        assert_eq!(get(&h.metrics().requests_rejected_invalid), 1);
    }

    #[tokio::test]
    async fn missing_or_wrong_content_type_is_415() {
        let h = harness();
        for ct in [None, Some("text/plain"), Some("application/protobuf")] {
            let (status, _, _) = send(&h, post("/v1/logs", Some(TOKEN), ct, vec![])).await;
            assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{ct:?}");
        }
        assert_eq!(get(&h.metrics().requests_rejected_invalid), 3);
    }

    #[tokio::test]
    async fn content_type_is_case_insensitive() {
        let h = harness();
        let body = traces(vec![good_span()]).encode_to_vec();
        let (status, _, _) = send(&h, post("/v1/traces", Some(TOKEN), Some("Application/X-Protobuf"), body)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = send(&h, events_req(Some(TOKEN), Some("APPLICATION/JSON"), json!({"events": []}))).await;
        assert_ne!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        // A type that merely starts with application/json is not JSON.
        let (status, _, _) = send(&h, events_req(Some(TOKEN), Some("application/jsonx"), json!({"events": []}))).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn content_type_parameters_are_allowed() {
        let h = harness();
        let body = traces(vec![good_span()]).encode_to_vec();
        let (status, _, _) =
            send(&h, post("/v1/traces", Some(TOKEN), Some("application/x-protobuf; charset=binary"), body)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn malformed_protobuf_is_400() {
        let h = harness();
        for path in ["/v1/traces", "/v1/logs", "/v1/metrics"] {
            let (status, headers, resp) = send(&h, post(path, Some(TOKEN), Some(PB), vec![0xff, 0xff, 0xff])).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            let s = rpc_status(&headers, &resp);
            assert_eq!(s.code, 3);
            assert!(s.message.starts_with("malformed"));
        }
        assert_eq!(get(&h.metrics().requests_rejected_invalid), 3);
        assert!(h.broker.records().is_empty());
    }

    #[tokio::test]
    async fn gzip_bodies_are_accepted() {
        let h = harness();
        let body = gzip(&traces(vec![good_span()]).encode_to_vec());
        let req = Request::post("/v1/traces")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, PB)
            .header(header::CONTENT_ENCODING, "gzip")
            .body(Body::from(body))
            .unwrap();
        let (status, _, _) = send(&h, req).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(published_traces(&h).len(), 1);
    }

    fn with_encoding(encoding: &str, body: Vec<u8>) -> Request<Body> {
        Request::post("/v1/traces")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, PB)
            .header(header::CONTENT_ENCODING, encoding)
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn invalid_gzip_is_400_and_unknown_encoding_is_415() {
        let h = harness();
        let (status, headers, resp) = send(&h, with_encoding("gzip", b"not gzip".to_vec())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(rpc_status(&headers, &resp).message.contains("gzip"));
        let (status, _, _) = send(&h, with_encoding("br", vec![])).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(get(&h.metrics().requests_rejected_invalid), 2);
    }

    #[tokio::test]
    async fn gzip_bomb_is_rejected_by_the_decompressed_limit() {
        let h = harness();
        let bomb = gzip(&vec![0u8; MAX_DECOMPRESSED_BODY + 1]);
        assert!(bomb.len() < MAX_COMPRESSED_BODY);
        let (status, headers, resp) = send(&h, with_encoding("gzip", bomb)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(rpc_status(&headers, &resp).message.contains("too large"));
        assert_eq!(get(&h.metrics().requests_rejected_invalid), 1);
    }

    #[tokio::test]
    async fn oversized_compressed_body_is_413() {
        let h = harness();
        let (status, _, _) =
            send(&h, post("/v1/traces", Some(TOKEN), Some(PB), vec![0u8; MAX_COMPRESSED_BODY + 1])).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(h.broker.records().is_empty());
    }

    #[test]
    fn decode_body_unit_cases() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(PB));
        assert_eq!(decode_body(&headers, &Bytes::from_static(b"abc")).unwrap(), b"abc");
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("identity"));
        assert_eq!(decode_body(&headers, &Bytes::from_static(b"abc")).unwrap(), b"abc");
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        assert_eq!(decode_body(&headers, &Bytes::from(gzip(b"hello"))).unwrap(), b"hello");
        let exact = gzip(&vec![1u8; MAX_DECOMPRESSED_BODY]);
        assert_eq!(decode_body(&headers, &Bytes::from(exact)).unwrap().len(), MAX_DECOMPRESSED_BODY);
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("deflate"));
        assert_eq!(decode_body(&headers, &Bytes::new()).unwrap_err().status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    // ---- accept path ----

    #[tokio::test]
    async fn accepted_traces_are_sanitised_and_scoped_by_the_token() {
        let h = harness();
        let body = traces(vec![good_span()]).encode_to_vec();
        let (status, headers, resp) = send(&h, post("/v1/traces", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()), Some(PB));
        let parsed = ExportTraceServiceResponse::decode(resp.as_slice()).unwrap();
        assert!(parsed.partial_success.is_none());

        let env = &h.envelopes()[0];
        assert_eq!(env.scope.as_ref().unwrap().tenant_id, "t1");
        let dump = format!("{:?}", published_traces(&h));
        assert!(!dump.contains("leaked-token"), "{dump}");
        assert_eq!(get(&h.metrics().spans_accepted), 1);
        assert_eq!(get(&h.metrics().attributes_redacted), 1);
        assert_eq!(get(&h.metrics().requests_accepted), 1);
    }

    #[tokio::test]
    async fn invalid_spans_yield_partial_success_and_are_counted() {
        let h = harness();
        let body = traces(vec![good_span(), bad_span(), bad_span()]).encode_to_vec();
        let (status, _, resp) = send(&h, post("/v1/traces", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::OK);
        let ps = ExportTraceServiceResponse::decode(resp.as_slice()).unwrap().partial_success.unwrap();
        assert_eq!(ps.rejected_spans, 2);
        assert!(!ps.error_message.is_empty());
        assert_eq!(get(&h.metrics().spans_accepted), 1);
        assert_eq!(get(&h.metrics().spans_rejected), 2);
        let published = published_traces(&h);
        assert_eq!(published[0].resource_spans[0].scope_spans[0].spans.len(), 1);
    }

    #[tokio::test]
    async fn all_invalid_spans_are_not_published() {
        let h = harness();
        let body = traces(vec![bad_span()]).encode_to_vec();
        let (status, _, resp) = send(&h, post("/v1/traces", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::OK);
        let ps = ExportTraceServiceResponse::decode(resp.as_slice()).unwrap().partial_success.unwrap();
        assert_eq!(ps.rejected_spans, 1);
        assert!(h.broker.records().is_empty());
        assert_eq!(get(&h.metrics().spans_rejected), 1);
    }

    #[tokio::test]
    async fn empty_export_is_ok_and_not_published() {
        let h = harness();
        let (status, _, resp) = send(&h, post("/v1/traces", Some(TOKEN), Some(PB), vec![])).await;
        assert_eq!(status, StatusCode::OK);
        assert!(ExportTraceServiceResponse::decode(resp.as_slice()).unwrap().partial_success.is_none());
        assert!(h.broker.records().is_empty());
    }

    #[tokio::test]
    async fn dropped_attributes_over_the_limit_are_counted() {
        let h = harness();
        let mut span = good_span();
        span.attributes = (0..70).map(|i| kv(&format!("k{i}"), "v")).collect();
        let body = traces(vec![span]).encode_to_vec();
        let (status, _, _) = send(&h, post("/v1/traces", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(get(&h.metrics().attributes_truncated), 6);
    }

    #[tokio::test]
    async fn broker_failure_is_503_never_200() {
        let h = harness();
        for mode in [Mode::Fail(PublishError::Unavailable("down".into())), Mode::Hang, Mode::Drop] {
            h.broker.set_mode(mode.clone());
            let body = traces(vec![good_span()]).encode_to_vec();
            let (status, headers, resp) = send(&h, post("/v1/traces", Some(TOKEN), Some(PB), body)).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{mode:?}");
            let s = rpc_status(&headers, &resp);
            assert_eq!(s.code, 14);
            assert!(s.message.contains("NOT accepted"));
            assert_eq!(retry_after(&headers), Some("2"));
        }
        assert_eq!(get(&h.metrics().requests_failed_broker), 3);
        assert_eq!(get(&h.metrics().spans_accepted), 0);
        assert_eq!(get(&h.metrics().requests_accepted), 0);
    }

    #[tokio::test]
    async fn overload_is_429_with_retry_after() {
        let h = harness();
        h.broker.set_mode(Mode::Fail(PublishError::Overloaded));
        let body = logs(vec![good_log()]).encode_to_vec();
        let (status, headers, resp) = send(&h, post("/v1/logs", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(rpc_status(&headers, &resp).code, 8);
        assert_eq!(retry_after(&headers), Some("1"));
        assert_eq!(get(&h.metrics().requests_rejected_overload), 1);
        assert_eq!(get(&h.metrics().logs_accepted), 0);
    }

    #[tokio::test]
    async fn logs_are_sanitised_and_partially_rejected() {
        let h = harness();
        let body = logs(vec![good_log(), LogRecord::default()]).encode_to_vec();
        let (status, _, resp) = send(&h, post("/v1/logs", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::OK);
        let ps = ExportLogsServiceResponse::decode(resp.as_slice()).unwrap().partial_success.unwrap();
        assert_eq!(ps.rejected_log_records, 1);
        assert_eq!(get(&h.metrics().logs_accepted), 1);
        assert_eq!(get(&h.metrics().logs_rejected), 1);
        let Some(Payload::OtlpLogs(bytes)) = h.envelopes().remove(0).payload else { panic!("expected logs") };
        let dump = format!("{:?}", ExportLogsServiceRequest::decode(bytes.as_slice()).unwrap());
        assert!(!dump.contains("hunter2") && !dump.contains("abc123"), "{dump}");
    }

    #[tokio::test]
    async fn logs_broker_failure_is_503() {
        let h = harness();
        h.broker.set_mode(Mode::Fail(PublishError::Timeout));
        let body = logs(vec![good_log()]).encode_to_vec();
        let (status, _, _) = send(&h, post("/v1/logs", Some(TOKEN), Some(PB), body)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(get(&h.metrics().logs_accepted), 0);
    }

    #[tokio::test]
    async fn accept_errors_map_to_otlp_status_codes() {
        let cases: [(AcceptError, StatusCode, i32, Option<&str>); 7] = [
            (AuthError::Missing.into(), StatusCode::UNAUTHORIZED, 16, None),
            (AuthError::Invalid.into(), StatusCode::UNAUTHORIZED, 16, None),
            (AuthError::SignalNotAllowed("logs").into(), StatusCode::FORBIDDEN, 7, None),
            (AuthError::Unavailable.into(), StatusCode::SERVICE_UNAVAILABLE, 14, Some("2")),
            (PublishError::Overloaded.into(), StatusCode::TOO_MANY_REQUESTS, 8, Some("1")),
            (PublishError::Timeout.into(), StatusCode::SERVICE_UNAVAILABLE, 14, Some("2")),
            (PublishError::Unavailable("x".into()).into(), StatusCode::SERVICE_UNAVAILABLE, 14, Some("2")),
        ];
        for (err, status, code, retry) in cases {
            let resp = accept_error_response(err);
            assert_eq!(resp.status(), status);
            assert_eq!(retry_after(resp.headers()), retry);
            let headers = resp.headers().clone();
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            assert_eq!(rpc_status(&headers, &body).code, code);
        }
    }

    // ---- events API ----

    fn event(id: &str) -> Json {
        json!({
            "event_id": id,
            "event_type": "deployment.started",
            "service": {"name": "orders"},
            "event_time": "1790000000000000000",
            "attributes": {"api_key": "sk-live"}
        })
    }

    fn events_req(token: Option<&str>, ct: Option<&str>, body: Json) -> Request<Body> {
        post("/api/v1/events/batch", token, ct, serde_json::to_vec(&body).unwrap())
    }

    fn json_body(body: &[u8]) -> Json {
        serde_json::from_slice(body).unwrap()
    }

    #[tokio::test]
    async fn events_are_published_with_broker_durability() {
        let h = harness();
        let body = json!({"events": [event("e1"), event("e2"), {"event_id": "bad"}]});
        let (status, _, resp) = send(&h, events_req(Some(TOKEN), Some("application/json"), body)).await;
        assert_eq!(status, StatusCode::OK);
        let v = json_body(&resp);
        assert_eq!(v["accepted"], 2);
        assert_eq!(v["rejected"][0]["index"], 2);
        assert_eq!(v["durability"], "broker_acknowledged");
        assert_eq!(v["batch_id"], h.envelopes()[0].batch_id.as_str());
        assert_eq!(get(&h.metrics().events_accepted), 2);
        assert_eq!(get(&h.metrics().events_rejected), 1);
        let Some(Payload::CustomEvents(batch)) = h.envelopes().remove(0).payload else { panic!("expected events") };
        assert_eq!(batch.events.len(), 2);
        assert!(!format!("{batch:?}").contains("sk-live"));
    }

    #[tokio::test]
    async fn events_charset_parameter_is_allowed() {
        let h = harness();
        let body = json!({"events": [event("e1")]});
        let (status, _, _) = send(&h, events_req(Some(TOKEN), Some("application/json; charset=utf-8"), body)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn events_auth_errors_are_json() {
        let h = harness();
        let body = json!({"events": [event("e1")]});
        let (status, _, resp) = send(&h, events_req(None, Some("application/json"), body.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(json_body(&resp)["error"].is_string());
        let (status, _, _) = send(&h, events_req(Some(TRACES_ONLY), Some("application/json"), body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        h.store.lock().unwrap().unavailable = true;
        let (status, _, _) = send(&h, events_req(Some("other"), Some("application/json"), body)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(h.broker.records().is_empty());
    }

    #[tokio::test]
    async fn events_require_json_content_type() {
        let h = harness();
        let (status, _, _) = send(&h, events_req(Some(TOKEN), Some("text/plain"), json!({"events": []}))).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let (status, _, _) = send(&h, events_req(Some(TOKEN), None, json!({"events": []}))).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn malformed_events_body_is_400_and_counted() {
        let h = harness();
        let req = post("/api/v1/events/batch", Some(TOKEN), Some("application/json"), "{not json");
        let (status, _, resp) = send(&h, req).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json_body(&resp)["error"].as_str().unwrap().starts_with("malformed JSON"));
        assert_eq!(get(&h.metrics().requests_rejected_invalid), 1);
    }

    #[tokio::test]
    async fn batch_with_no_valid_events_is_400_and_not_published() {
        let h = harness();
        let body = json!({"events": [{"event_id": "x"}, {"event_id": "y"}]});
        let (status, _, resp) = send(&h, events_req(Some(TOKEN), Some("application/json"), body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let v = json_body(&resp);
        assert_eq!(v["accepted"], 0);
        assert_eq!(v["rejected"].as_array().map(Vec::len), Some(2));
        assert_eq!(get(&h.metrics().events_rejected), 2);
        assert!(h.broker.records().is_empty());
    }

    #[tokio::test]
    async fn events_broker_failures_never_report_success() {
        let h = harness();
        h.broker.set_mode(Mode::Fail(PublishError::Unavailable("down".into())));
        let body = json!({"events": [event("e1")]});
        let (status, headers, resp) = send(&h, events_req(Some(TOKEN), Some("application/json"), body.clone())).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(retry_after(&headers), Some("2"));
        assert!(json_body(&resp)["error"].as_str().unwrap().contains("NOT accepted"));

        h.broker.set_mode(Mode::Fail(PublishError::Overloaded));
        let (status, headers, _) = send(&h, events_req(Some(TOKEN), Some("application/json"), body)).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(retry_after(&headers), Some("1"));
        assert_eq!(get(&h.metrics().events_accepted), 0);
    }
}
