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
