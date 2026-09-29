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
enum CredentialStore {
    Pg(PgStore),
    #[cfg(test)]
    Fake(Arc<std::sync::Mutex<fake::FakeStore>>),
}

impl CredentialStore {
    async fn find(&self, hash: &[u8]) -> Result<Option<IngestCredential>, AuthError> {
        match self {
            CredentialStore::Pg(pg) => pg.find_ingest_token(hash).await.map_err(|e| {
                tracing::error!(error = %e, "ingest token lookup failed");
                AuthError::Unavailable
            }),
            #[cfg(test)]
            CredentialStore::Fake(store) => store.lock().map_err(|_| AuthError::Unavailable)?.find(hash),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Authenticator {
    store: CredentialStore,
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
        Self { store: CredentialStore::Pg(pg), cache: Arc::new(RwLock::new(HashMap::new())) }
    }

    #[cfg(test)]
    pub(crate) fn fake(store: Arc<std::sync::Mutex<fake::FakeStore>>) -> Self {
        Self { store: CredentialStore::Fake(store), cache: Arc::new(RwLock::new(HashMap::new())) }
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
                let fetched = self.store.find(&hash).await?;
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
pub(crate) mod fake {
    use super::*;

    /// In-memory stand-in for the PostgreSQL token table.
    #[derive(Debug, Default)]
    pub(crate) struct FakeStore {
        pub tokens: HashMap<Vec<u8>, IngestCredential>,
        pub unavailable: bool,
        pub lookups: usize,
    }

    impl FakeStore {
        pub fn insert(&mut self, token: &str, cred: IngestCredential) {
            self.tokens.insert(hash_token(token), cred);
        }

        pub fn find(&mut self, hash: &[u8]) -> Result<Option<IngestCredential>, AuthError> {
            self.lookups += 1;
            if self.unavailable {
                return Err(AuthError::Unavailable);
            }
            Ok(self.tokens.get(hash).cloned())
        }
    }

    pub(crate) fn credential(tenant: &str, signals: &[&str]) -> IngestCredential {
        use controlhub_domain::{EnvironmentId, ProjectId, Scope, TenantId};
        IngestCredential {
            token_id: uuid::Uuid::now_v7(),
            scope: Scope {
                tenant_id: TenantId::parse(tenant).unwrap(),
                project_id: ProjectId::parse("p1").unwrap(),
                environment_id: EnvironmentId::parse("dev").unwrap(),
            },
            signals: signals.iter().map(|s| (*s).to_owned()).collect(),
            expires_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeStore, credential};
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn parses_bearer() {
        assert_eq!(bearer(Some("Bearer abc")).ok(), Some("abc"));
        assert_eq!(bearer(Some("bearer  abc ")).ok(), Some("abc"));
        assert!(bearer(Some("Basic abc")).is_err());
        assert!(bearer(Some("Bearer ")).is_err());
        assert!(bearer(None).is_err());
    }

    #[test]
    fn bearer_rejects_malformed_headers() {
        for h in ["", "Bearer", "Bearerabc", "Token abc", " Bearer abc", "Bearer \t ", "BEARER"] {
            assert!(matches!(bearer(Some(h)), Err(AuthError::Missing)), "{h:?}");
        }
        assert_eq!(bearer(Some("BEARER abc")).ok(), Some("abc"));
        assert_eq!(bearer(Some("Bearer a.b-c_d~e")).ok(), Some("a.b-c_d~e"));
    }

    #[test]
    fn signal_names_match_token_grants() {
        assert_eq!(signal_name(Signal::Traces), "traces");
        assert_eq!(signal_name(Signal::Logs), "logs");
        assert_eq!(signal_name(Signal::Metrics), "metrics");
        assert_eq!(signal_name(Signal::Events), "events");
    }

    fn setup(signals: &[&str]) -> (Authenticator, Arc<Mutex<FakeStore>>) {
        let store = Arc::new(Mutex::new(FakeStore::default()));
        store.lock().unwrap().insert("good", credential("t1", signals));
        (Authenticator::fake(store.clone()), store)
    }

    #[tokio::test]
    async fn valid_token_resolves_scope_from_the_store() {
        let (auth, _) = setup(&["traces"]);
        let cred = auth.authenticate("good", Signal::Traces).await.unwrap();
        assert_eq!(cred.scope.tenant_id.as_str(), "t1");
    }

    #[tokio::test]
    async fn unknown_token_is_invalid() {
        let (auth, _) = setup(&["traces"]);
        assert!(matches!(auth.authenticate("nope", Signal::Traces).await, Err(AuthError::Invalid)));
    }

    #[tokio::test]
    async fn signal_not_granted_is_forbidden() {
        let (auth, _) = setup(&["traces"]);
        assert!(matches!(auth.authenticate("good", Signal::Logs).await, Err(AuthError::SignalNotAllowed("logs"))));
        assert!(matches!(auth.authenticate("good", Signal::Events).await, Err(AuthError::SignalNotAllowed("events"))));
    }

    #[tokio::test]
    async fn expired_token_is_invalid_even_when_cached() {
        let store = Arc::new(Mutex::new(FakeStore::default()));
        let mut cred = credential("t1", &["traces"]);
        cred.expires_at = Some(time_now() - std::time::Duration::from_secs(1));
        store.lock().unwrap().insert("old", cred);
        let auth = Authenticator::fake(store.clone());
        assert!(matches!(auth.authenticate("old", Signal::Traces).await, Err(AuthError::Invalid)));
        assert!(matches!(auth.authenticate("old", Signal::Traces).await, Err(AuthError::Invalid)));
        assert_eq!(store.lock().unwrap().lookups, 1, "second call served from cache");
    }

    #[tokio::test]
    async fn future_expiry_is_accepted() {
        let store = Arc::new(Mutex::new(FakeStore::default()));
        let mut cred = credential("t1", &["logs"]);
        cred.expires_at = Some(time_now() + std::time::Duration::from_secs(3_600));
        store.lock().unwrap().insert("tok", cred);
        let auth = Authenticator::fake(store);
        assert!(auth.authenticate("tok", Signal::Logs).await.is_ok());
    }

    #[tokio::test]
    async fn positive_and_negative_results_are_cached() {
        let (auth, store) = setup(&["traces"]);
        for _ in 0..3 {
            auth.authenticate("good", Signal::Traces).await.unwrap();
            let _ = auth.authenticate("bad", Signal::Traces).await;
        }
        assert_eq!(store.lock().unwrap().lookups, 2);
    }

    #[tokio::test]
    async fn store_outage_is_unavailable_and_not_cached() {
        let (auth, store) = setup(&["traces"]);
        store.lock().unwrap().unavailable = true;
        assert!(matches!(auth.authenticate("good", Signal::Traces).await, Err(AuthError::Unavailable)));
        store.lock().unwrap().unavailable = false;
        assert!(auth.authenticate("good", Signal::Traces).await.is_ok(), "outage must not poison the cache");
    }

    #[tokio::test]
    async fn cache_is_keyed_by_hash_not_plaintext() {
        let (auth, _) = setup(&["traces"]);
        auth.authenticate("good", Signal::Traces).await.unwrap();
        let cache = auth.cache.read().await;
        assert!(cache.contains_key(&hash_token("good")));
        assert!(!cache.keys().any(|k| k.as_slice() == b"good"));
    }

    #[tokio::test]
    async fn cache_is_bounded() {
        let (auth, _) = setup(&["traces"]);
        {
            let mut cache = auth.cache.write().await;
            for i in 0..MAX_ENTRIES {
                cache.insert(format!("k{i}").into_bytes(), (None, Instant::now()));
            }
        }
        auth.authenticate("good", Signal::Traces).await.unwrap();
        assert_eq!(auth.cache.read().await.len(), 1, "full cache is cleared before inserting");
    }
}
