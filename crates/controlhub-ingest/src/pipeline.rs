//! Shared accept path for all transports: authenticate -> sanitise ->
//! wrap in envelope -> publish -> ack.

use std::collections::BTreeMap;
use std::sync::Arc;

use controlhub_domain::Signal;
use controlhub_protocol::ENVELOPE_SCHEMA_VERSION;
use controlhub_protocol::v1::{IngestEnvelope, IngestScope, ingest_envelope::Payload};
use controlhub_storage::pg::IngestCredential;
use prost::Message;

use crate::auth::{AuthError, Authenticator};
use crate::broker::{PublishError, Publisher};
use crate::metrics::Metrics;

#[derive(Debug, Clone)]
pub struct AppState {
    pub auth: Authenticator,
    pub publisher: Publisher,
    pub metrics: Arc<Metrics>,
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
