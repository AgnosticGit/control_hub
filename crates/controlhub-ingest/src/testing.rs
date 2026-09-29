//! Test harness: an [`AppState`] wired to the in-memory credential store and broker.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use controlhub_protocol::v1::IngestEnvelope;
use prost::Message;

use crate::auth::Authenticator;
use crate::auth::fake::{FakeStore, credential};
use crate::broker::PublisherConfig;
use crate::broker::fake::{FakeBroker, publisher};
use crate::metrics::Metrics;
use crate::pipeline::AppState;

/// Token allowed to send traces, logs and events for tenant `t1`.
pub(crate) const TOKEN: &str = "chi_test_all_signals";
/// Token allowed to send traces only.
pub(crate) const TRACES_ONLY: &str = "chi_test_traces_only";

#[derive(Debug)]
pub(crate) struct Harness {
    pub state: AppState,
    pub broker: FakeBroker,
    pub store: Arc<Mutex<FakeStore>>,
}

impl Harness {
    pub fn metrics(&self) -> &Metrics {
        &self.state.metrics
    }

    /// Envelopes the broker acknowledged, decoded.
    pub fn envelopes(&self) -> Vec<IngestEnvelope> {
        self.broker
            .records()
            .into_iter()
            .map(|(_, r)| IngestEnvelope::decode(r.value.unwrap_or_default().as_slice()).unwrap())
            .collect()
    }
}

pub(crate) fn get(c: &AtomicU64) -> u64 {
    c.load(Ordering::Relaxed)
}

pub(crate) fn harness() -> Harness {
    let metrics = Arc::new(Metrics::default());
    let store = Arc::new(Mutex::new(FakeStore::default()));
    {
        let mut s = store.lock().unwrap();
        s.insert(TOKEN, credential("t1", &["traces", "logs", "events"]));
        s.insert(TRACES_ONLY, credential("t1", &["traces"]));
    }
    let cfg = PublisherConfig { ack_timeout: Duration::from_millis(300), ..PublisherConfig::default() };
    let (publisher, broker) = publisher(2, cfg, metrics.clone());
    let state =
        AppState { auth: Authenticator::fake(store.clone()), publisher, metrics, source_id: "ingest-test".into() };
    Harness { state, broker, store }
}
