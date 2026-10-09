//! Bounded, in-process fan-out for live timeline items.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use karbird_domain::Scope;
use karbird_engine_core::TimelineItem;
use tokio::sync::mpsc;

const HISTORY_WINDOW_NS: i64 = 30_000_000_000;
const MAX_HISTORY_ITEMS_PER_SCOPE: usize = 256;
const MAX_SCOPES: usize = 128;
const DEFAULT_SUBSCRIBER_CAPACITY: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct LiveItem {
    pub cursor_ns: i64,
    pub item: TimelineItem,
}

#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error("live hub lock is poisoned")]
    LockPoisoned,
    #[error("live hub has reached its active scope limit")]
    AtCapacity,
}

#[derive(Debug)]
struct ScopeChannel {
    history: VecDeque<LiveItem>,
    subscribers: Vec<mpsc::Sender<Vec<LiveItem>>>,
    last_cursor_ns: i64,
}

#[derive(Debug, Default)]
struct HubCounters {
    subscriber_overflows: AtomicU64,
    scope_evictions: AtomicU64,
    capacity_rejections: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HubMetrics {
    pub subscriber_overflows: u64,
    pub scope_evictions: u64,
    pub capacity_rejections: u64,
}

#[derive(Debug, Clone)]
pub struct LiveHub {
    scopes: Arc<Mutex<HashMap<Scope, ScopeChannel>>>,
    subscriber_capacity: usize,
    counters: Arc<HubCounters>,
}

impl Default for LiveHub {
    fn default() -> Self {
        Self::new(DEFAULT_SUBSCRIBER_CAPACITY)
    }
}

impl LiveHub {
    pub fn new(subscriber_capacity: usize) -> Self {
        Self {
            scopes: Arc::new(Mutex::new(HashMap::new())),
            subscriber_capacity: subscriber_capacity.max(1),
            counters: Arc::new(HubCounters::default()),
        }
    }

    pub fn metrics(&self) -> HubMetrics {
        HubMetrics {
            subscriber_overflows: self.counters.subscriber_overflows.load(Ordering::Relaxed),
            scope_evictions: self.counters.scope_evictions.load(Ordering::Relaxed),
            capacity_rejections: self.counters.capacity_rejections.load(Ordering::Relaxed),
        }
    }

    /// Subscribe to one exact tenant/project/environment and return its recent replay window.
    pub fn subscribe(
        &self,
        scope: Scope,
        after_cursor_ns: i64,
    ) -> Result<(Vec<LiveItem>, mpsc::Receiver<Vec<LiveItem>>), HubError> {
        let mut scopes = self.scopes.lock().map_err(|_| HubError::LockPoisoned)?;
        scopes.retain(|_, channel| !channel.subscribers.is_empty() || !channel.history.is_empty());
        if !scopes.contains_key(&scope) && scopes.len() >= MAX_SCOPES {
            if let Some(oldest) = scopes
                .iter()
                .filter(|(_, channel)| channel.subscribers.is_empty())
                .min_by_key(|(_, channel)| channel.last_cursor_ns)
                .map(|(scope, _)| scope.clone())
            {
                scopes.remove(&oldest);
                self.counters.scope_evictions.fetch_add(1, Ordering::Relaxed);
            } else {
                self.counters.capacity_rejections.fetch_add(1, Ordering::Relaxed);
                return Err(HubError::AtCapacity);
            }
        }
        let channel = scopes.entry(scope).or_insert_with(|| ScopeChannel {
            history: VecDeque::new(),
            subscribers: Vec::new(),
            last_cursor_ns: after_cursor_ns,
        });
        let (tx, rx) = mpsc::channel(self.subscriber_capacity);
        let history = channel.history.iter().filter(|item| item.cursor_ns > after_cursor_ns).cloned().collect();
        channel.subscribers.push(tx);
        Ok((history, rx))
    }

    /// Retain a bounded replay window and enqueue the item for each live subscriber.
    /// Full subscriber queues are closed so their WebSocket can emit a gap and recover via HTTP.
    #[cfg(test)]
    pub fn publish(&self, scope: Scope, item: LiveItem) -> Result<(), HubError> {
        self.publish_batch(scope, vec![item])
    }

    pub fn publish_batch(&self, scope: Scope, items: Vec<LiveItem>) -> Result<(), HubError> {
        if items.is_empty() {
            return Ok(());
        }
        let mut scopes = self.scopes.lock().map_err(|_| HubError::LockPoisoned)?;
        scopes.retain(|_, channel| !channel.subscribers.is_empty() || !channel.history.is_empty());
        if !scopes.contains_key(&scope) && scopes.len() >= MAX_SCOPES {
            // Do not stall the Kafka reader behind a scope-cardinality spike. Evicting the
            // oldest active scope closes its bounded WS queues; those sockets emit `gap` and
            // recover their window over the authorized HTTP path.
            let oldest =
                scopes.iter().min_by_key(|(_, channel)| channel.last_cursor_ns).map(|(scope, _)| scope.clone());
            if let Some(oldest) = oldest {
                scopes.remove(&oldest);
                self.counters.scope_evictions.fetch_add(1, Ordering::Relaxed);
            } else {
                self.counters.capacity_rejections.fetch_add(1, Ordering::Relaxed);
                return Err(HubError::AtCapacity);
            }
        }
        let channel = scopes.entry(scope).or_insert_with(|| ScopeChannel {
            history: VecDeque::new(),
            subscribers: Vec::new(),
            last_cursor_ns: items.last().map_or(0, |item| item.cursor_ns),
        });
        for item in &items {
            let cutoff = item.cursor_ns.saturating_sub(HISTORY_WINDOW_NS);
            while channel.history.front().is_some_and(|old| old.cursor_ns < cutoff) {
                channel.history.pop_front();
            }
            channel.history.push_back(item.clone());
            while channel.history.len() > MAX_HISTORY_ITEMS_PER_SCOPE {
                channel.history.pop_front();
            }
        }
        channel.last_cursor_ns = items.last().map_or(channel.last_cursor_ns, |item| item.cursor_ns);
        channel.subscribers.retain(|subscriber| match subscriber.try_send(items.clone()) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.counters.subscriber_overflows.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use karbird_domain::{EnvironmentId, ProjectId, TenantId};
    use karbird_engine_core::LogItem;

    fn scope(tenant: &str, environment: &str) -> Scope {
        Scope {
            tenant_id: TenantId::parse(tenant).unwrap(),
            project_id: ProjectId::parse("project").unwrap(),
            environment_id: EnvironmentId::parse(environment).unwrap(),
        }
    }

    fn item(cursor_ns: i64, id: &str) -> LiveItem {
        LiveItem {
            cursor_ns,
            item: TimelineItem::Log(LogItem {
                id: id.into(),
                service_id: "svc".into(),
                service_name: "service".into(),
                time_ns: cursor_ns,
                severity: 9,
                trace_id: None,
            }),
        }
    }

    #[test]
    fn subscribers_only_receive_exact_scope_items() {
        let hub = LiveHub::new(8);
        let (mut history_a, mut rx_a) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();
        let (mut history_b, mut rx_b) = hub.subscribe(scope("tenant-b", "prod"), 0).unwrap();
        assert!(history_a.is_empty() && history_b.is_empty());

        hub.publish(scope("tenant-a", "prod"), item(1, "a1")).unwrap();

        assert_eq!(rx_a.try_recv().unwrap()[0].item, item(1, "a1").item);
        assert!(rx_b.try_recv().is_err());
        history_a.clear();
        history_b.clear();
    }

    #[test]
    fn reconnect_replays_only_items_after_the_resume_cursor() {
        let hub = LiveHub::new(8);
        hub.publish(scope("tenant-a", "prod"), item(10, "a1")).unwrap();
        hub.publish(scope("tenant-a", "prod"), item(20, "a2")).unwrap();

        let (replay, _rx) = hub.subscribe(scope("tenant-a", "prod"), 10).unwrap();

        assert_eq!(replay, vec![item(20, "a2")]);
    }

    #[test]
    fn one_envelope_chunk_is_enqueued_atomically_and_replayed() {
        let hub = LiveHub::new(1);
        let (replay, mut rx) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();
        let items = vec![item(10, "a1"), item(10, "a2")];
        hub.publish_batch(scope("tenant-a", "prod"), items.clone()).unwrap();
        assert_eq!(rx.try_recv().unwrap(), items);
        assert_eq!(replay, Vec::<LiveItem>::new());
        let (replay, _) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();
        assert_eq!(replay.len(), 2);
    }

    #[tokio::test]
    async fn full_subscriber_is_closed_without_blocking_other_subscribers() {
        let hub = LiveHub::new(1);
        let (_, mut slow) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();
        let (_, mut healthy) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();

        hub.publish(scope("tenant-a", "prod"), item(1, "a1")).unwrap();
        assert_eq!(healthy.recv().await.unwrap()[0].item, item(1, "a1").item);
        hub.publish(scope("tenant-a", "prod"), item(2, "a2")).unwrap();

        assert!(slow.recv().await.is_some());
        assert!(slow.recv().await.is_none(), "overflow closes the subscriber so WS can signal a gap");
        assert_eq!(healthy.recv().await.unwrap()[0].item, item(2, "a2").item);
        assert_eq!(hub.metrics().subscriber_overflows, 1);
    }

    #[test]
    fn scope_cardinality_pressure_disconnects_oldest_stream_instead_of_blocking_publish() {
        let hub = LiveHub::new(8);
        let mut receivers = Vec::new();
        for index in 0..MAX_SCOPES {
            let scope = scope("tenant-a", &format!("env-{index}"));
            let (_, rx) = hub.subscribe(scope, 0).unwrap();
            receivers.push(rx);
        }
        hub.publish_batch(scope("tenant-a", "env-new"), vec![item(10, "new")]).unwrap();
        assert!(receivers.iter_mut().any(|rx| matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Disconnected))));
        assert_eq!(hub.metrics().scope_evictions, 1);
    }
}
