//! Shared accept path for all transports: authenticate -> sanitise ->
//! wrap in envelope -> publish -> ack.

use std::collections::BTreeMap;
use std::sync::Arc;

use karbird_domain::Signal;
use karbird_protocol::ENVELOPE_SCHEMA_VERSION;
use karbird_protocol::otlp::collector::metrics::v1::ExportMetricsServiceRequest;
use karbird_protocol::v1::{IngestEnvelope, IngestScope, ingest_envelope::Payload};
use karbird_storage::pg::IngestCredential;
use prost::Message;

use crate::auth::{AuthError, Authenticator};
use crate::broker::{PublishError, Publisher};
use crate::metrics::Metrics;
use crate::sanitize::MetricStats;
use crate::series::SeriesLimiter;

#[derive(Debug, Clone)]
pub struct AppState {
    pub auth: Authenticator,
    pub publisher: Publisher,
    pub metrics: Arc<Metrics>,
    pub series: Arc<SeriesLimiter>,
    pub source_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AcceptError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Publish(#[from] PublishError),
}

#[derive(Debug, Clone)]
pub struct Accepted {
    pub batch_id: String,
}

fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
}

impl AppState {
    pub async fn authenticate(&self, header: Option<&str>, signal: Signal) -> Result<IngestCredential, AcceptError> {
        let token = crate::auth::bearer(header).inspect_err(|_| {
            Metrics::inc(&self.metrics.requests_rejected_auth, 1);
        })?;
        self.auth.authenticate(token, signal).await.map_err(|e| {
            if !matches!(e, AuthError::Unavailable) {
                Metrics::inc(&self.metrics.requests_rejected_auth, 1);
            }
            AcceptError::Auth(e)
        })
    }

    /// Sanitises metrics under the credential's series budget and counts every outcome.
    pub fn sanitize_metrics(&self, cred: &IngestCredential, req: &mut ExportMetricsServiceRequest) -> MetricStats {
        let key = format!("{}/{}/{}", cred.scope.tenant_id, cred.scope.project_id, cred.scope.environment_id);
        let stats = crate::sanitize::sanitize_metrics(req, &mut |series| self.series.admit(&key, series));
        Metrics::inc(&self.metrics.attributes_redacted, stats.redacted);
        Metrics::inc(&self.metrics.attributes_truncated, stats.truncated_attrs);
        Metrics::inc(&self.metrics.metric_points_rejected, stats.rejected);
        Metrics::inc(&self.metrics.metric_series_rejected, stats.over_limit);
        stats
    }

    /// Publish a sanitised payload under the credential's scope. The payload's
    /// own resource attributes can never change the tenant/project/environment.
    pub async fn publish(&self, cred: &IngestCredential, payload: Payload) -> Result<Accepted, AcceptError> {
        let batch_id = uuid::Uuid::now_v7().to_string();
        let envelope = IngestEnvelope {
            schema_version: ENVELOPE_SCHEMA_VERSION,
            batch_id: batch_id.clone(),
            scope: Some(IngestScope {
                tenant_id: cred.scope.tenant_id.to_string(),
                project_id: cred.scope.project_id.to_string(),
                environment_id: cred.scope.environment_id.to_string(),
                token_id: cred.token_id.to_string(),
            }),
            received_time_unix_nano: now_ns(),
            source_id: self.source_id.clone(),
            payload: Some(payload),
        };
        let key = format!("{}/{}/{}", cred.scope.tenant_id, cred.scope.project_id, cred.scope.environment_id);
        let headers = BTreeMap::from([("schema_version".to_owned(), ENVELOPE_SCHEMA_VERSION.to_string().into_bytes())]);
        match self.publisher.publish(&key, envelope.encode_to_vec(), headers).await {
            Ok(_offset) => {
                Metrics::inc(&self.metrics.requests_accepted, 1);
                Ok(Accepted { batch_id })
            }
            Err(e) => {
                match e {
                    PublishError::Overloaded => Metrics::inc(&self.metrics.requests_rejected_overload, 1),
                    _ => Metrics::inc(&self.metrics.requests_failed_broker, 1),
                }
                Err(e.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::fake::Mode;
    use crate::testing::{TOKEN, TRACES_ONLY, get, harness};

    #[tokio::test]
    async fn missing_or_invalid_tokens_are_counted() {
        let h = harness();
        let st = &h.state;
        assert!(matches!(st.authenticate(None, Signal::Traces).await, Err(AcceptError::Auth(AuthError::Missing))));
        assert!(matches!(
            st.authenticate(Some("Basic abc"), Signal::Traces).await,
            Err(AcceptError::Auth(AuthError::Missing))
        ));
        assert!(matches!(
            st.authenticate(Some("Bearer wrong"), Signal::Traces).await,
            Err(AcceptError::Auth(AuthError::Invalid))
        ));
        assert!(matches!(
            st.authenticate(Some(&format!("Bearer {TRACES_ONLY}")), Signal::Logs).await,
            Err(AcceptError::Auth(AuthError::SignalNotAllowed("logs")))
        ));
        assert_eq!(get(&h.metrics().requests_rejected_auth), 4);
    }

    #[tokio::test]
    async fn store_outage_is_not_counted_as_an_auth_rejection() {
        let h = harness();
        h.store.lock().unwrap().unavailable = true;
        let r = h.state.authenticate(Some(&format!("Bearer {TOKEN}")), Signal::Traces).await;
        assert!(matches!(r, Err(AcceptError::Auth(AuthError::Unavailable))));
        assert_eq!(get(&h.metrics().requests_rejected_auth), 0);
    }

    #[tokio::test]
    async fn envelope_scope_comes_from_the_credential() {
        let h = harness();
        let cred = h.state.authenticate(Some(&format!("Bearer {TOKEN}")), Signal::Traces).await.unwrap();
        let accepted = h.state.publish(&cred, Payload::OtlpTraces(vec![1, 2, 3])).await.unwrap();

        let records = h.broker.records();
        assert_eq!(records.len(), 1);
        let rec = &records[0].1;
        assert_eq!(rec.key.as_deref(), Some(&b"t1/p1/dev"[..]));
        assert_eq!(rec.headers.get("schema_version").map(Vec::as_slice), Some(&b"1"[..]));

        let env = &h.envelopes()[0];
        assert_eq!(env.schema_version, ENVELOPE_SCHEMA_VERSION);
        assert_eq!(env.batch_id, accepted.batch_id);
        assert_eq!(uuid::Uuid::parse_str(&env.batch_id).unwrap().get_version_num(), 7);
        let scope = env.scope.as_ref().unwrap();
        assert_eq!(
            (scope.tenant_id.as_str(), scope.project_id.as_str(), scope.environment_id.as_str()),
            ("t1", "p1", "dev")
        );
        assert_eq!(scope.token_id, cred.token_id.to_string());
        assert_eq!(env.source_id, "ingest-test");
        assert!(env.received_time_unix_nano > 1_700_000_000_000_000_000);
        assert_eq!(env.payload, Some(Payload::OtlpTraces(vec![1, 2, 3])));
        assert_eq!(get(&h.metrics().requests_accepted), 1);
    }

    #[tokio::test]
    async fn batch_ids_are_unique() {
        let h = harness();
        let cred = h.state.authenticate(Some(&format!("Bearer {TOKEN}")), Signal::Logs).await.unwrap();
        let a = h.state.publish(&cred, Payload::OtlpLogs(vec![])).await.unwrap();
        let b = h.state.publish(&cred, Payload::OtlpLogs(vec![])).await.unwrap();
        assert_ne!(a.batch_id, b.batch_id);
    }

    #[tokio::test]
    async fn broker_errors_are_propagated_and_counted() {
        let h = harness();
        let cred = h.state.authenticate(Some(&format!("Bearer {TOKEN}")), Signal::Traces).await.unwrap();

        h.broker.set_mode(Mode::Fail(PublishError::Unavailable("down".into())));
        let r = h.state.publish(&cred, Payload::OtlpTraces(vec![])).await;
        assert!(matches!(r, Err(AcceptError::Publish(PublishError::Unavailable(_)))));

        h.broker.set_mode(Mode::Hang);
        let r = h.state.publish(&cred, Payload::OtlpTraces(vec![])).await;
        assert!(matches!(r, Err(AcceptError::Publish(PublishError::Timeout))));

        h.broker.set_mode(Mode::Fail(PublishError::Overloaded));
        let r = h.state.publish(&cred, Payload::OtlpTraces(vec![])).await;
        assert!(matches!(r, Err(AcceptError::Publish(PublishError::Overloaded))));

        assert_eq!(get(&h.metrics().requests_failed_broker), 2);
        assert_eq!(get(&h.metrics().requests_rejected_overload), 1);
        assert_eq!(get(&h.metrics().requests_accepted), 0);
        assert!(h.broker.records().is_empty());
    }
}
