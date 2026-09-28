//! Ingestion token authentication with a short-lived bounded cache.
//! Revocation takes effect within `CACHE_TTL`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use controlhub_domain::Signal;
use controlhub_storage::pg::{IngestCredential, PgStore};
use controlhub_storage::secrets::hash_token;
use tokio::sync::RwLock;

const CACHE_TTL: Duration = Duration::from_secs(30);
const NEGATIVE_TTL: Duration = Duration::from_secs(5);
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, thiserror::Error)]
pub enum AuthError {
    #[error("missing bearer token")]
    Missing,
    #[error("invalid or revoked token")]
    Invalid,
    #[error("token is not allowed to send {0}")]
    SignalNotAllowed(&'static str),
    #[error("credential store unavailable")]
    Unavailable,
}

type CredentialCache = HashMap<Vec<u8>, (Option<IngestCredential>, Instant)>;

#[derive(Debug, Clone)]
pub struct Authenticator {
    pg: PgStore,
    cache: Arc<RwLock<CredentialCache>>,
}

pub fn bearer(header: Option<&str>) -> Result<&str, AuthError> {
    let h = header.ok_or(AuthError::Missing)?;
    let (scheme, token) = h.split_once(' ').ok_or(AuthError::Missing)?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.trim().is_empty() {
        return Err(AuthError::Missing);
    }
    Ok(token.trim())
}

fn signal_name(s: Signal) -> &'static str {
    match s {
        Signal::Traces => "traces",
        Signal::Logs => "logs",
        Signal::Metrics => "metrics",
        Signal::Events => "events",
    }
}

impl Authenticator {
    pub fn new(pg: PgStore) -> Self {
        Self { pg, cache: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub async fn authenticate(&self, token: &str, signal: Signal) -> Result<IngestCredential, AuthError> {
        let hash = hash_token(token);
        let cached = {
            let cache = self.cache.read().await;
            cache.get(&hash).and_then(|(cred, at)| {
                let ttl = if cred.is_some() { CACHE_TTL } else { NEGATIVE_TTL };
                (at.elapsed() < ttl).then(|| cred.clone())
            })
        };
        let cred = match cached {
            Some(c) => c,
            None => {
                let fetched = self.pg.find_ingest_token(&hash).await.map_err(|e| {
                    tracing::error!(error = %e, "ingest token lookup failed");
                    AuthError::Unavailable
                })?;
                let mut cache = self.cache.write().await;
                if cache.len() >= MAX_ENTRIES {
                    cache.clear();
                }
                cache.insert(hash, (fetched.clone(), Instant::now()));
                fetched
            }
        };
        let cred = cred.ok_or(AuthError::Invalid)?;
        if cred.expires_at.is_some_and(|e| e <= time_now()) {
            return Err(AuthError::Invalid);
        }
        let name = signal_name(signal);
        if !cred.signals.iter().any(|s| s == name) {
            return Err(AuthError::SignalNotAllowed(name));
        }
        Ok(cred)
    }
}

fn time_now() -> controlhub_storage::pg::OffsetDateTimeAlias {
    controlhub_storage::pg::OffsetDateTimeAlias::now_utc()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bearer() {
        assert_eq!(bearer(Some("Bearer abc")).ok(), Some("abc"));
        assert_eq!(bearer(Some("bearer  abc ")).ok(), Some("abc"));
        assert!(bearer(Some("Basic abc")).is_err());
        assert!(bearer(Some("Bearer ")).is_err());
        assert!(bearer(None).is_err());
    }
}
