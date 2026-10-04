//! OTLP/gRPC: standard unary `Export` RPCs for traces, logs and metrics
//! (gauges and sums only, ADR-0018).

use karbird_domain::Signal;
use karbird_protocol::otlp::collector::logs::v1::logs_service_server::LogsService;
use karbird_protocol::otlp::collector::logs::v1::{
    ExportLogsPartialSuccess, ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use karbird_protocol::otlp::collector::metrics::v1::metrics_service_server::MetricsService;
use karbird_protocol::otlp::collector::metrics::v1::{ExportMetricsServiceRequest, ExportMetricsServiceResponse};
use karbird_protocol::otlp::collector::trace::v1::trace_service_server::TraceService;
use karbird_protocol::otlp::collector::trace::v1::{
    ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use karbird_protocol::v1::ingest_envelope::Payload;
use prost::Message;
use tonic::{Request, Response, Status};

use crate::auth::AuthError;
use crate::broker::PublishError;
use crate::http::metrics_partial_success;
use crate::metrics::Metrics;
use crate::pipeline::{AcceptError, AppState};
use crate::sanitize::{sanitize_logs, sanitize_traces};

#[derive(Debug, Clone)]
pub struct OtlpGrpc {
    pub state: AppState,
}

fn to_status(e: AcceptError) -> Status {
    match e {
        AcceptError::Auth(AuthError::Missing | AuthError::Invalid) => {
            Status::unauthenticated("invalid or missing ingestion token")
        }
        AcceptError::Auth(AuthError::SignalNotAllowed(s)) => {
            Status::permission_denied(format!("token not allowed to send {s}"))
        }
        AcceptError::Auth(AuthError::Unavailable) => Status::unavailable("credential store unavailable"),
        // OTLP: RESOURCE_EXHAUSTED is retryable only with RetryInfo; UNAVAILABLE is always retryable.
        AcceptError::Publish(PublishError::Overloaded) => Status::unavailable("ingest queue full, retry later"),
        AcceptError::Publish(_) => {
            Status::unavailable("broker did not acknowledge the write; data NOT accepted, retry")
        }
    }
}

fn auth_header<T>(req: &Request<T>) -> Option<String> {
    req.metadata().get("authorization").and_then(|v| v.to_str().ok()).map(str::to_owned)
}

#[tonic::async_trait]
impl TraceService for OtlpGrpc {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let st = &self.state;
        let header = auth_header(&request);
        let cred = st.authenticate(header.as_deref(), Signal::Traces).await.map_err(to_status)?;
        let mut req = request.into_inner();
        let stats = sanitize_traces(&mut req);
        Metrics::inc(&st.metrics.attributes_redacted, stats.redacted);
        Metrics::inc(&st.metrics.attributes_truncated, stats.truncated_attrs);
        Metrics::inc(&st.metrics.spans_rejected, stats.rejected);
        if stats.accepted > 0 {
            st.publish(&cred, Payload::OtlpTraces(req.encode_to_vec())).await.map_err(to_status)?;
            Metrics::inc(&st.metrics.spans_accepted, stats.accepted);
        }
        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: (stats.rejected > 0).then(|| ExportTracePartialSuccess {
                rejected_spans: stats.rejected as i64,
                error_message: "spans with invalid ids/timestamps or over the per-request limit were rejected".into(),
            }),
        }))
    }
}

#[tonic::async_trait]
impl LogsService for OtlpGrpc {
    async fn export(
        &self,
        request: Request<ExportLogsServiceRequest>,
    ) -> Result<Response<ExportLogsServiceResponse>, Status> {
        let st = &self.state;
        let header = auth_header(&request);
        let cred = st.authenticate(header.as_deref(), Signal::Logs).await.map_err(to_status)?;
        let mut req = request.into_inner();
        let stats = sanitize_logs(&mut req);
        Metrics::inc(&st.metrics.attributes_redacted, stats.redacted);
        Metrics::inc(&st.metrics.attributes_truncated, stats.truncated_attrs);
        Metrics::inc(&st.metrics.logs_rejected, stats.rejected);
        if stats.accepted > 0 {
            st.publish(&cred, Payload::OtlpLogs(req.encode_to_vec())).await.map_err(to_status)?;
            Metrics::inc(&st.metrics.logs_accepted, stats.accepted);
        }
        Ok(Response::new(ExportLogsServiceResponse {
            partial_success: (stats.rejected > 0).then(|| ExportLogsPartialSuccess {
                rejected_log_records: stats.rejected as i64,
                error_message: "log records with invalid ids/timestamps or over the per-request limit were rejected"
                    .into(),
            }),
        }))
    }
}

#[tonic::async_trait]
impl MetricsService for OtlpGrpc {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        let st = &self.state;
        let header = auth_header(&request);
        let cred = st.authenticate(header.as_deref(), Signal::Metrics).await.map_err(to_status)?;
        let mut req = request.into_inner();
        let stats = st.sanitize_metrics(&cred, &mut req);
        if stats.accepted > 0 {
            st.publish(&cred, Payload::OtlpMetrics(req.encode_to_vec())).await.map_err(to_status)?;
            Metrics::inc(&st.metrics.metric_points_accepted, stats.accepted);
        }
        Ok(Response::new(ExportMetricsServiceResponse { partial_success: metrics_partial_success(&stats) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::fake::Mode;
    use crate::testing::{Harness, TOKEN, TRACES_ONLY, get, harness};
    use karbird_protocol::otlp::common::v1::{AnyValue, KeyValue, any_value::Value};
    use karbird_protocol::otlp::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use karbird_protocol::otlp::metrics::v1::{
        Gauge, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Summary, SummaryDataPoint, metric::Data,
        number_data_point,
    };

    fn metrics(token: Option<&str>, with_summary: bool) -> Request<ExportMetricsServiceRequest> {
        let gauge = Metric {
            name: "system.memory.usage".into(),
            data: Some(Data::Gauge(Gauge {
                data_points: vec![NumberDataPoint {
                    attributes: vec![kv("state", "used"), kv("password", "pw-secret")],
                    time_unix_nano: 1,
                    value: Some(number_data_point::Value::AsInt(870 << 20)),
                    ..Default::default()
                }],
            })),
            ..Default::default()
        };
        let mut list = vec![gauge];
        if with_summary {
            list.push(Metric {
                name: "rpc.duration".into(),
                data: Some(Data::Summary(Summary { data_points: vec![SummaryDataPoint::default()] })),
                ..Default::default()
            });
        }
        let mut req = Request::new(ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics { metrics: list, ..Default::default() }],
                ..Default::default()
            }],
        });
        if let Some(t) = token {
            req.metadata_mut().insert("authorization", format!("Bearer {t}").parse().unwrap());
        }
        req
    }

    #[tokio::test]
    async fn metrics_are_accepted_over_grpc() {
        let h = harness();
        let resp = MetricsService::export(&svc(&h), metrics(Some(TOKEN), true)).await.unwrap();
        assert_eq!(resp.into_inner().partial_success.map(|p| p.rejected_data_points), Some(1));
        let envs = h.envelopes();
        assert!(matches!(envs[0].payload, Some(Payload::OtlpMetrics(_))));
        assert!(!format!("{envs:?}").contains("pw-secret"));
        assert_eq!(get(&h.metrics().metric_points_accepted), 1);
        assert_eq!(get(&h.metrics().metric_points_rejected), 1);
    }

    #[tokio::test]
    async fn metrics_auth_and_broker_failures_over_grpc() {
        let h = harness();
        let err = MetricsService::export(&svc(&h), metrics(None, false)).await.unwrap_err();
        assert_eq!(err.code(), Code::Unauthenticated);
        let err = MetricsService::export(&svc(&h), metrics(Some(TRACES_ONLY), false)).await.unwrap_err();
        assert_eq!(err.code(), Code::PermissionDenied);
        h.broker.set_mode(Mode::Fail(PublishError::Unavailable("down".into())));
        let err = MetricsService::export(&svc(&h), metrics(Some(TOKEN), false)).await.unwrap_err();
        assert_eq!(err.code(), Code::Unavailable);
        assert_eq!(get(&h.metrics().metric_points_accepted), 0);
    }
    use karbird_protocol::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use tonic::Code;

    fn kv(k: &str, v: &str) -> KeyValue {
        KeyValue {
            key: k.into(),
            value: Some(AnyValue { value: Some(Value::StringValue(v.into())) }),
            ..Default::default()
        }
    }

    fn span(valid: bool) -> Span {
        Span {
            trace_id: vec![u8::from(valid); 16],
            span_id: vec![2; 8],
            start_time_unix_nano: 1,
            end_time_unix_nano: 2,
            attributes: vec![kv("cookie", "sid=secret-cookie")],
            ..Default::default()
        }
    }

    fn traces(spans: Vec<Span>, token: Option<&str>) -> Request<ExportTraceServiceRequest> {
        let mut req = Request::new(ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans { spans, ..Default::default() }],
                ..Default::default()
            }],
        });
        if let Some(t) = token {
            req.metadata_mut().insert("authorization", format!("Bearer {t}").parse().unwrap());
        }
        req
    }

    fn logs(records: Vec<LogRecord>, token: Option<&str>) -> Request<ExportLogsServiceRequest> {
        let mut req = Request::new(ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs { log_records: records, ..Default::default() }],
                ..Default::default()
            }],
        });
        if let Some(t) = token {
            req.metadata_mut().insert("authorization", format!("Bearer {t}").parse().unwrap());
        }
        req
    }

    fn log(time: u64) -> LogRecord {
        LogRecord { time_unix_nano: time, attributes: vec![kv("api_key", "sk-secret")], ..Default::default() }
    }

    fn svc(h: &Harness) -> OtlpGrpc {
        OtlpGrpc { state: h.state.clone() }
    }

    #[tokio::test]
    async fn traces_are_accepted_redacted_and_published() {
        let h = harness();
        let resp = TraceService::export(&svc(&h), traces(vec![span(true)], Some(TOKEN))).await.unwrap();
        assert!(resp.into_inner().partial_success.is_none());
        assert_eq!(h.envelopes().len(), 1);
        let dump = format!("{:?}", h.envelopes());
        assert!(!dump.contains("secret-cookie"));
        assert_eq!(get(&h.metrics().spans_accepted), 1);
        assert_eq!(get(&h.metrics().attributes_redacted), 1);
    }

    #[tokio::test]
    async fn invalid_spans_are_reported_as_partial_success() {
        let h = harness();
        let resp = TraceService::export(&svc(&h), traces(vec![span(true), span(false)], Some(TOKEN))).await.unwrap();
        assert_eq!(resp.into_inner().partial_success.map(|p| p.rejected_spans), Some(1));
        assert_eq!(get(&h.metrics().spans_rejected), 1);

        let resp = TraceService::export(&svc(&h), traces(vec![span(false)], Some(TOKEN))).await.unwrap();
        assert_eq!(resp.into_inner().partial_success.map(|p| p.rejected_spans), Some(1));
        assert_eq!(h.envelopes().len(), 1, "nothing to publish when every span is invalid");
    }

    #[tokio::test]
    async fn auth_failures_map_to_grpc_codes() {
        let h = harness();
        let err = TraceService::export(&svc(&h), traces(vec![span(true)], None)).await.unwrap_err();
        assert_eq!(err.code(), Code::Unauthenticated);
        let err = TraceService::export(&svc(&h), traces(vec![span(true)], Some("wrong"))).await.unwrap_err();
        assert_eq!(err.code(), Code::Unauthenticated);
        let err = LogsService::export(&svc(&h), logs(vec![log(1)], Some(TRACES_ONLY))).await.unwrap_err();
        assert_eq!(err.code(), Code::PermissionDenied);
        h.store.lock().unwrap().unavailable = true;
        let err = LogsService::export(&svc(&h), logs(vec![log(1)], Some("fresh"))).await.unwrap_err();
        assert_eq!(err.code(), Code::Unavailable);
        assert!(h.broker.records().is_empty());
        assert_eq!(get(&h.metrics().requests_rejected_auth), 3);
    }

    #[tokio::test]
    async fn broker_failures_are_unavailable_never_ok() {
        let h = harness();
        for mode in [
            Mode::Fail(PublishError::Unavailable("down".into())),
            Mode::Fail(PublishError::Overloaded),
            Mode::Hang,
            Mode::Drop,
        ] {
            h.broker.set_mode(mode.clone());
            let err = TraceService::export(&svc(&h), traces(vec![span(true)], Some(TOKEN))).await.unwrap_err();
            assert_eq!(err.code(), Code::Unavailable, "{mode:?}");
            let err = LogsService::export(&svc(&h), logs(vec![log(1)], Some(TOKEN))).await.unwrap_err();
            assert_eq!(err.code(), Code::Unavailable, "{mode:?}");
        }
        assert_eq!(get(&h.metrics().spans_accepted), 0);
        assert_eq!(get(&h.metrics().logs_accepted), 0);
        assert_eq!(get(&h.metrics().requests_accepted), 0);
    }

    #[tokio::test]
    async fn logs_are_accepted_and_partially_rejected() {
        let h = harness();
        let resp = LogsService::export(&svc(&h), logs(vec![log(1), log(0)], Some(TOKEN))).await.unwrap();
        assert_eq!(resp.into_inner().partial_success.map(|p| p.rejected_log_records), Some(1));
        assert_eq!(get(&h.metrics().logs_accepted), 1);
        assert_eq!(get(&h.metrics().logs_rejected), 1);
        assert!(!format!("{:?}", h.envelopes()).contains("sk-secret"));
    }

    #[test]
    fn status_mapping() {
        let cases: [(AcceptError, Code); 7] = [
            (AuthError::Missing.into(), Code::Unauthenticated),
            (AuthError::Invalid.into(), Code::Unauthenticated),
            (AuthError::SignalNotAllowed("traces").into(), Code::PermissionDenied),
            (AuthError::Unavailable.into(), Code::Unavailable),
            (PublishError::Overloaded.into(), Code::Unavailable),
            (PublishError::Timeout.into(), Code::Unavailable),
            (PublishError::Unavailable("x".into()).into(), Code::Unavailable),
        ];
        for (err, code) in cases {
            assert_eq!(to_status(err).code(), code);
        }
    }

    #[test]
    fn auth_header_reads_metadata() {
        let mut req = Request::new(());
        assert_eq!(auth_header(&req), None);
        req.metadata_mut().insert("authorization", "Bearer x".parse().unwrap());
        assert_eq!(auth_header(&req).as_deref(), Some("Bearer x"));
    }
}
