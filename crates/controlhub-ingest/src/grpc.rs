//! OTLP/gRPC: standard unary `Export` RPCs for traces and logs.
//! The metrics service is not registered, so clients get `UNIMPLEMENTED`.

use controlhub_domain::Signal;
use controlhub_protocol::otlp::collector::logs::v1::logs_service_server::LogsService;
use controlhub_protocol::otlp::collector::logs::v1::{
    ExportLogsPartialSuccess, ExportLogsServiceRequest, ExportLogsServiceResponse,
};
use controlhub_protocol::otlp::collector::trace::v1::trace_service_server::TraceService;
use controlhub_protocol::otlp::collector::trace::v1::{
    ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use controlhub_protocol::v1::ingest_envelope::Payload;
use prost::Message;
use tonic::{Request, Response, Status};

use crate::auth::AuthError;
use crate::broker::PublishError;
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
