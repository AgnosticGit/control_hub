//! Bounded, in-process fan-out for live timeline items.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::routes::telemetry::TopologyResponse;
use karbird_domain::Scope;
use karbird_engine_core::{InfrastructureSnapshot, TimelineItem};
use serde::Serialize;
use tokio::sync::mpsc;

const HISTORY_WINDOW_NS: i64 = 30_000_000_000;
const MAX_HISTORY_ITEMS_PER_SCOPE: usize = 256;
const MAX_SCOPES: usize = 128;
const INFRA_CACHE_TTL: Duration = Duration::from_secs(10);
const DEFAULT_SUBSCRIBER_CAPACITY: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct LiveItem {
    pub cursor_ns: i64,
    pub storage_cursor_ns: i64,
    pub item: TimelineItem,
    pub signal: Option<LiveSignal>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LiveUpdate {
    Telemetry(Box<LiveItem>),
    Action(serde_json::Value),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "record", rename_all = "snake_case")]
pub enum LiveSignal {
    Span(crate::routes::telemetry::SpanDto),
    Log(crate::routes::telemetry::LogDto),
    Event(crate::routes::telemetry::EventDto),
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
    subscribers: Vec<LiveSubscriber>,
    last_cursor_ns: i64,
}

#[derive(Debug)]
struct LiveSubscriber {
    sender: mpsc::Sender<LiveUpdate>,
    actions_enabled: bool,
}

#[derive(Debug, Default)]
struct HubCounters {
    subscriber_overflows: AtomicU64,
    scope_evictions: AtomicU64,
    capacity_rejections: AtomicU64,
}

type InfrastructureCache = Arc<Mutex<HashMap<Scope, Arc<tokio::sync::Mutex<Option<CachedInfrastructure>>>>>>;
type TopologyCache = Arc<Mutex<HashMap<Scope, Arc<tokio::sync::Mutex<Option<CachedTopology>>>>>>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HubMetrics {
    pub subscriber_overflows: u64,
    pub scope_evictions: u64,
    pub capacity_rejections: u64,
}

#[derive(Debug, Clone)]
pub struct LiveHub {
    scopes: Arc<Mutex<HashMap<Scope, ScopeChannel>>>,
    infrastructure: InfrastructureCache,
    topology: TopologyCache,
    subscriber_capacity: usize,
    counters: Arc<HubCounters>,
}

#[derive(Debug)]
struct CachedInfrastructure {
    refreshed: Instant,
    snapshot: Arc<InfrastructureSnapshot>,
}

#[derive(Debug)]
struct CachedTopology {
    refreshed: Instant,
    snapshot: Arc<TopologyResponse>,
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
            infrastructure: Arc::new(Mutex::new(HashMap::new())),
            topology: Arc::new(Mutex::new(HashMap::new())),
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

    /// Coalesces live infrastructure reads across sockets in one scope for ten seconds.
    /// If the bounded cache is full or poisoned, loading still succeeds without caching.
    pub async fn infrastructure_snapshot<E, F, Fut>(
        &self,
        scope: &Scope,
        load: F,
    ) -> Result<Arc<InfrastructureSnapshot>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<InfrastructureSnapshot, E>>,
    {
        let Some(slot) = self.infrastructure_slot(scope) else {
            return load().await.map(Arc::new);
        };
        let mut cached = slot.lock().await;
        if let Some(cached) = cached.as_ref().filter(|cached| cached.refreshed.elapsed() < INFRA_CACHE_TTL) {
            return Ok(Arc::clone(&cached.snapshot));
        }
        let snapshot = Arc::new(load().await?);
        *cached = Some(CachedInfrastructure { refreshed: Instant::now(), snapshot: Arc::clone(&snapshot) });
        Ok(snapshot)
    }

    /// Coalesces backend-owned topology reads across live sockets for ten seconds per scope.
    pub async fn topology_snapshot<E, F, Fut>(&self, scope: &Scope, load: F) -> Result<Arc<TopologyResponse>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<TopologyResponse, E>>,
    {
        let Some(slot) = self.topology_slot(scope) else {
            return load().await.map(Arc::new);
        };
        let mut cached = slot.lock().await;
        if let Some(cached) = cached.as_ref().filter(|cached| cached.refreshed.elapsed() < INFRA_CACHE_TTL) {
            return Ok(Arc::clone(&cached.snapshot));
        }
        let snapshot = Arc::new(load().await?);
        *cached = Some(CachedTopology { refreshed: Instant::now(), snapshot: Arc::clone(&snapshot) });
        Ok(snapshot)
    }

    fn infrastructure_slot(&self, scope: &Scope) -> Option<Arc<tokio::sync::Mutex<Option<CachedInfrastructure>>>> {
        let mut entries = self.infrastructure.lock().ok()?;
        if let Some(slot) = entries.get(scope) {
            return Some(Arc::clone(slot));
        }
        if entries.len() >= MAX_SCOPES {
            entries.retain(|_, slot| Arc::strong_count(slot) > 1);
        }
        if entries.len() >= MAX_SCOPES {
            return None;
        }
        let slot = Arc::new(tokio::sync::Mutex::new(None));
        entries.insert(scope.clone(), Arc::clone(&slot));
        Some(slot)
    }

    fn topology_slot(&self, scope: &Scope) -> Option<Arc<tokio::sync::Mutex<Option<CachedTopology>>>> {
        let mut entries = self.topology.lock().ok()?;
        if let Some(slot) = entries.get(scope) {
            return Some(Arc::clone(slot));
        }
        if entries.len() >= MAX_SCOPES {
            entries.retain(|_, slot| Arc::strong_count(slot) > 1);
        }
        if entries.len() >= MAX_SCOPES {
            return None;
        }
        let slot = Arc::new(tokio::sync::Mutex::new(None));
        entries.insert(scope.clone(), Arc::clone(&slot));
        Some(slot)
    }

    /// Subscribe to one exact tenant/project/environment and return its recent replay window.
    #[cfg(test)]
    pub fn subscribe(
        &self,
        scope: Scope,
        after_cursor_ns: i64,
    ) -> Result<(Vec<LiveItem>, mpsc::Receiver<LiveUpdate>), HubError> {
        self.subscribe_with_actions(scope, after_cursor_ns, false)
    }

    pub fn subscribe_with_actions(
        &self,
        scope: Scope,
        after_cursor_ns: i64,
        actions_enabled: bool,
    ) -> Result<(Vec<LiveItem>, mpsc::Receiver<LiveUpdate>), HubError> {
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
        channel.subscribers.push(LiveSubscriber { sender: tx, actions_enabled });
        Ok((history, rx))
    }

    /// Retain a bounded replay window and enqueue each item as its own live message.
    /// Full subscriber queues are closed so their WebSocket reconnects from its resume cursor.
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
        for item in items {
            channel.subscribers.retain(|subscriber| {
                match subscriber.sender.try_send(LiveUpdate::Telemetry(Box::new(item.clone()))) {
                    Ok(()) => true,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        self.counters.subscriber_overflows.fetch_add(1, Ordering::Relaxed);
                        false
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => false,
                }
            });
        }
        Ok(())
    }

    /// Fan out one committed action-plan DTO to subscribers in the exact authorized scope.
    pub fn publish_action(&self, scope: Scope, plan: serde_json::Value) -> Result<(), HubError> {
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
            } else {
                self.counters.capacity_rejections.fetch_add(1, Ordering::Relaxed);
                return Err(HubError::AtCapacity);
            }
        }
        let channel = scopes.entry(scope).or_insert_with(|| ScopeChannel {
            history: VecDeque::new(),
            subscribers: Vec::new(),
            last_cursor_ns: 0,
        });
        channel.subscribers.retain(|subscriber| {
            if !subscriber.actions_enabled {
                return true;
            }
            match subscriber.sender.try_send(LiveUpdate::Action(plan.clone())) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    self.counters.subscriber_overflows.fetch_add(1, Ordering::Relaxed);
                    false
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
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
            storage_cursor_ns: cursor_ns,
            signal: None,
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

    #[tokio::test]
    async fn infrastructure_snapshots_are_cached_per_exact_scope() {
        use std::sync::atomic::AtomicUsize;

        let hub = LiveHub::default();
        let calls = Arc::new(AtomicUsize::new(0));
        for expected in [1, 1] {
            let calls_for_load = Arc::clone(&calls);
            let snapshot = hub
                .infrastructure_snapshot(&scope("tenant-a", "prod"), || async move {
                    calls_for_load.fetch_add(1, Ordering::Relaxed);
                    Ok::<_, ()>(karbird_engine_core::InfrastructureSnapshot::default())
                })
                .await
                .unwrap();
            assert_eq!(snapshot.hosts.len(), 0);
            assert_eq!(calls.load(Ordering::Relaxed), expected);
        }

        let calls_for_other_scope = Arc::clone(&calls);
        hub.infrastructure_snapshot(&scope("tenant-a", "stage"), || async move {
            calls_for_other_scope.fetch_add(1, Ordering::Relaxed);
            Ok::<_, ()>(karbird_engine_core::InfrastructureSnapshot::default())
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn concurrent_infrastructure_requests_share_one_load() {
        use std::sync::atomic::AtomicUsize;

        let hub = LiveHub::default();
        let scope = scope("tenant-a", "prod");
        let calls = Arc::new(AtomicUsize::new(0));
        let load = |calls: Arc<AtomicUsize>| async move {
            calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok::<_, ()>(InfrastructureSnapshot::default())
        };
        let (first, second) = tokio::join!(
            hub.infrastructure_snapshot(&scope, || load(Arc::clone(&calls))),
            hub.infrastructure_snapshot(&scope, || load(Arc::clone(&calls))),
        );

        assert!(first.is_ok() && second.is_ok());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn topology_reads_are_coalesced_per_scope() {
        use std::sync::atomic::AtomicUsize;

        let hub = LiveHub::default();
        let scope = scope("tenant-a", "prod");
        let calls = Arc::new(AtomicUsize::new(0));
        let load = |calls: Arc<AtomicUsize>| async move {
            calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok::<_, ()>(crate::routes::telemetry::TopologyResponse {
                nodes: vec![],
                edges: vec![],
                from_ns: "1".into(),
                to_ns: "2".into(),
                completeness: crate::routes::telemetry::Completeness {
                    watermark_ns: None,
                    processing_caught_up: true,
                    unresolved_processing_errors: 0,
                    truncated: false,
                    sampling: "unknown".into(),
                    notes: vec![],
                },
                geometry: karbird_engine_core::SavedTopologyGeometry {
                    version: karbird_engine_core::SERVER_LAYOUT_VERSION,
                    revision: 1,
                    plain: Default::default(),
                    infrastructure: None,
                    external: vec![],
                },
                updated_at_ns: "2".into(),
                infrastructure_snapshot: None,
            })
        };
        let (first, second) = tokio::join!(
            hub.topology_snapshot(&scope, || load(Arc::clone(&calls))),
            hub.topology_snapshot(&scope, || load(Arc::clone(&calls))),
        );
        assert!(first.is_ok() && second.is_ok());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn subscribers_only_receive_exact_scope_items() {
        let hub = LiveHub::new(8);
        let (mut history_a, mut rx_a) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();
        let (mut history_b, mut rx_b) = hub.subscribe(scope("tenant-b", "prod"), 0).unwrap();
        assert!(history_a.is_empty() && history_b.is_empty());

        hub.publish(scope("tenant-a", "prod"), item(1, "a1")).unwrap();

        assert_eq!(rx_a.try_recv().unwrap(), LiveUpdate::Telemetry(Box::new(item(1, "a1"))));
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
    fn every_item_is_enqueued_as_its_own_message_and_replayed() {
        let hub = LiveHub::new(4);
        let (replay, mut rx) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();
        let items = vec![item(10, "a1"), item(10, "a2")];
        hub.publish_batch(scope("tenant-a", "prod"), items.clone()).unwrap();
        assert_eq!(rx.try_recv().unwrap(), LiveUpdate::Telemetry(Box::new(items[0].clone())));
        assert_eq!(rx.try_recv().unwrap(), LiveUpdate::Telemetry(Box::new(items[1].clone())));
        assert_eq!(replay, Vec::<LiveItem>::new());
        let (replay, _) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();
        assert_eq!(replay.len(), 2);
    }

    #[test]
    fn action_updates_are_fanned_out_only_to_the_exact_scope() {
        let hub = LiveHub::new(4);
        let (_, mut prod) = hub.subscribe_with_actions(scope("tenant-a", "prod"), 0, true).unwrap();
        let (_, mut stage) = hub.subscribe_with_actions(scope("tenant-a", "stage"), 0, true).unwrap();
        let (_, mut telemetry_only) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();
        let plan = serde_json::json!({"id":"plan-1", "state":"running"});
        hub.publish_action(scope("tenant-a", "prod"), plan.clone()).unwrap();
        assert_eq!(prod.try_recv().unwrap(), LiveUpdate::Action(plan));
        assert!(stage.try_recv().is_err());
        assert!(telemetry_only.try_recv().is_err(), "telemetry viewers do not receive action DTOs");
    }

    #[tokio::test]
    async fn full_subscriber_is_closed_without_blocking_other_subscribers() {
        let hub = LiveHub::new(1);
        let (_, mut slow) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();
        let (_, mut healthy) = hub.subscribe(scope("tenant-a", "prod"), 0).unwrap();

        hub.publish(scope("tenant-a", "prod"), item(1, "a1")).unwrap();
        assert_eq!(healthy.recv().await.unwrap(), LiveUpdate::Telemetry(Box::new(item(1, "a1"))));
        hub.publish(scope("tenant-a", "prod"), item(2, "a2")).unwrap();

        assert!(slow.recv().await.is_some());
        assert!(slow.recv().await.is_none(), "overflow closes the subscriber so WS can signal a gap");
        assert_eq!(healthy.recv().await.unwrap(), LiveUpdate::Telemetry(Box::new(item(2, "a2"))));
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
