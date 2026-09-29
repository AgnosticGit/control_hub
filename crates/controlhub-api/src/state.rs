use std::sync::Arc;

use controlhub_storage::ch::ChStore;
use controlhub_storage::pg::PgStore;
use tokio::sync::Semaphore;

use crate::auth::LoginThrottle;

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub allowed_origins: Vec<String>,
    /// `Secure` attribute on cookies. Disabled only for local http development.
    pub secure_cookies: bool,
    pub session_ttl: time::Duration,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub pg: PgStore,
    pub ch: ChStore,
    pub config: ApiConfig,
    pub login_throttle: LoginThrottle,
    /// Bounds concurrent live WebSocket connections (each polls ClickHouse once per second).
    pub live_slots: Arc<Semaphore>,
}

/// State whose stores point at closed local ports: any handler that reaches storage fails
/// with a storage error, so tests prove which paths are decided before storage is touched.
/// Must be created inside a Tokio runtime.
#[cfg(test)]
pub fn test_state(secure_cookies: bool) -> AppState {
    let pg =
        PgStore::connect_lazy("postgres://u:p@127.0.0.1:1/controlhub", 1).unwrap_or_else(|e| panic!("lazy pool: {e}"));
    let ch = ChStore::new(&controlhub_storage::ch::ChConfig {
        url: "http://127.0.0.1:1".into(),
        database: "controlhub".into(),
        user: "u".into(),
        password: "p".into(),
    });
    AppState {
        pg,
        ch,
        config: ApiConfig {
            allowed_origins: vec!["http://localhost:3000".into(), "http://127.0.0.1:3000".into()],
            secure_cookies,
            session_ttl: time::Duration::hours(12),
        },
        login_throttle: LoginThrottle::default(),
        live_slots: Arc::new(Semaphore::new(2)),
    }
}
