use std::sync::Arc;

use karbird_storage::archive::ArchiveReader;
use karbird_storage::ch::ChStore;
use karbird_storage::pg::PgStore;
use tokio::sync::Semaphore;

use crate::auth::LoginThrottle;
use crate::routes::signup::SignupThrottle;
use crate::mail::Mailer;
use crate::routes::oauth::{GoogleOAuthConfig, OAuthStateStore};

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub allowed_origins: Vec<String>,
    /// `Secure` attribute on cookies. Disabled only for local http development.
    pub secure_cookies: bool,
    pub session_ttl: time::Duration,
    pub signup: SignupPolicy,
}

#[derive(Debug, Clone, Copy)]
pub struct SignupPolicy {
    /// Self-service sign-up is off unless explicitly enabled.
    pub open: bool,
    /// Sign-up attempts allowed per hour across all clients.
    pub per_hour: u32,
}

/// Log archive in object storage (ADR-0015). Without it the API serves ClickHouse only.
#[derive(Debug, Clone)]
pub struct ArchiveConfig {
    pub reader: ArchiveReader,
    /// `source=auto` reads logs older than `now - hot_window` from the archive. Shorter than the
    /// ClickHouse retention, so a range is never read from a table that has already trimmed it.
    pub hot_window_ns: i64,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub pg: PgStore,
    pub ch: ChStore,
    pub archive: Option<ArchiveConfig>,
    pub config: ApiConfig,
    pub login_throttle: LoginThrottle,
    pub signup_throttle: SignupThrottle,
    /// Bounds concurrent live WebSocket connections (each polls ClickHouse once per second).
    pub live_slots: Arc<Semaphore>,
    pub mailer: Arc<dyn Mailer>,
    pub public_url: String,
    pub google_oauth: Option<GoogleOAuthConfig>,
    pub oauth_states: OAuthStateStore,
    pub oauth_client: reqwest::Client,
}

/// State whose stores point at closed local ports: any handler that reaches storage fails
/// with a storage error, so tests prove which paths are decided before storage is touched.
/// Must be created inside a Tokio runtime.
#[cfg(test)]
pub fn test_state(secure_cookies: bool) -> AppState {
    let pg =
        PgStore::connect_lazy("postgres://u:p@127.0.0.1:1/karbird", 1).unwrap_or_else(|e| panic!("lazy pool: {e}"));
    let ch = ChStore::new(&karbird_storage::ch::ChConfig {
        url: "http://127.0.0.1:1".into(),
        database: "karbird".into(),
        user: "u".into(),
        password: "p".into(),
    });
    AppState {
        pg,
        ch,
        archive: None,
        config: ApiConfig {
            allowed_origins: vec!["http://localhost:3000".into(), "http://127.0.0.1:3000".into()],
            secure_cookies,
            session_ttl: time::Duration::hours(12),
            signup: SignupPolicy { open: true, per_hour: 1000 },
        },
        login_throttle: LoginThrottle::default(),
        signup_throttle: SignupThrottle::default(),
        live_slots: Arc::new(Semaphore::new(2)),
        mailer: Arc::new(crate::mail::DisabledMailer),
        public_url: "http://localhost:3000".into(),
        google_oauth: None,
        oauth_states: OAuthStateStore::default(),
        oauth_client: reqwest::Client::new(),
    }
}
