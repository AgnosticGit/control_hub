//! Broker publisher with bounded per-partition queues and request batching.
//!
//! A request is acknowledged to the client only after the broker confirmed
//! the write (`acks=all` in rskafka). The in-memory queue is only a staging
//! area: nothing in it has been acknowledged yet.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use controlhub_runtime::backoff;
use rskafka::client::Client;
use rskafka::client::partition::{Compression, PartitionClient, UnknownTopicHandling};
use rskafka::record::Record;
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::metrics::Metrics;

#[derive(Debug, Clone, thiserror::Error)]
pub enum PublishError {
    #[error("ingest queue is full")]
    Overloaded,
    #[error("broker unavailable: {0}")]
    Unavailable(String),
    #[error("timed out waiting for broker acknowledgement")]
    Timeout,
}

#[derive(Debug, Clone)]
pub struct PublisherConfig {
    pub queue_capacity: usize,
    pub max_queued_bytes: usize,
    pub max_batch_records: usize,
    pub max_batch_bytes: usize,
    pub linger: Duration,
    pub attempt_timeout: Duration,
    pub max_attempts: u32,
    pub ack_timeout: Duration,
}

impl Default for PublisherConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 512,
            max_queued_bytes: 128 * 1024 * 1024,
            max_batch_records: 256,
            max_batch_bytes: 4 * 1024 * 1024,
            linger: Duration::from_millis(5),
            attempt_timeout: Duration::from_secs(5),
            max_attempts: 3,
            ack_timeout: Duration::from_secs(15),
        }
    }
}

struct Job {
    record: Record,
    size: usize,
    respond: oneshot::Sender<Result<i64, PublishError>>,
}

#[derive(Debug, Clone)]
pub struct Publisher {
    inner: Arc<Inner>,
}

struct Inner {
    queues: Vec<mpsc::Sender<Job>>,
    bytes: Arc<Semaphore>,
    cfg: PublisherConfig,
    metrics: Arc<Metrics>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner").field("partitions", &self.queues.len()).finish()
    }
}

impl Publisher {
    pub async fn start(
        client: Arc<Client>,
        topic: &str,
        partitions: i32,
        cfg: PublisherConfig,
        metrics: Arc<Metrics>,
    ) -> anyhow::Result<(Self, Vec<JoinHandle<()>>)> {
        let mut queues = Vec::new();
        let mut handles = Vec::new();
        for p in 0..partitions {
            let pc = client.partition_client(topic.to_owned(), p, UnknownTopicHandling::Retry).await?;
            let (tx, rx) = mpsc::channel(cfg.queue_capacity);
            queues.push(tx);
            handles.push(tokio::spawn(run_partition(pc, rx, cfg.clone(), metrics.clone())));
        }
        let bytes = Arc::new(Semaphore::new(cfg.max_queued_bytes));
        Ok((Self { inner: Arc::new(Inner { queues, bytes, cfg, metrics }) }, handles))
    }

    /// Publish one envelope. Records with the same `key` land on the same
    /// partition (per-scope ordering). Returns the broker offset.
    pub async fn publish(
        &self,
        key: &str,
        value: Vec<u8>,
        headers: BTreeMap<String, Vec<u8>>,
    ) -> Result<i64, PublishError> {
        let inner = &self.inner;
        let size = value.len() + key.len();
        let permits = u32::try_from(size.min(inner.cfg.max_queued_bytes)).map_err(|_| PublishError::Overloaded)?;
        // Bounded memory: reject instead of buffering without limit.
        let _permit =
            inner.bytes.clone().try_acquire_many_owned(permits.max(1)).map_err(|_| PublishError::Overloaded)?;

        let idx = partition_for(key, inner.queues.len());
        let queue = inner.queues.get(idx).ok_or_else(|| PublishError::Unavailable("no partitions".into()))?;
        Metrics::inc(&inner.metrics.queued_bytes, size as u64);

        let (tx, rx) = oneshot::channel();
        let job = Job {
            record: Record {
                key: Some(key.as_bytes().to_vec()),
                value: Some(value),
                headers,
                timestamp: chrono::Utc::now(),
            },
            size,
            respond: tx,
        };
        let result = match queue.try_send(job) {
            Err(_) => Err(PublishError::Overloaded),
            Ok(()) => match tokio::time::timeout(inner.cfg.ack_timeout, rx).await {
                Ok(Ok(r)) => r,
                Ok(Err(_)) => Err(PublishError::Unavailable("publisher stopped".into())),
                Err(_) => Err(PublishError::Timeout),
            },
        };
        inner.metrics.queued_bytes.fetch_sub(size as u64, std::sync::atomic::Ordering::Relaxed);
        result
    }
}

/// Stable queue index for a record key (per-scope ordering).
fn partition_for(key: &str, partitions: usize) -> usize {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    (h.finish() % partitions.max(1) as u64) as usize
}

async fn run_partition(pc: PartitionClient, mut rx: mpsc::Receiver<Job>, cfg: PublisherConfig, metrics: Arc<Metrics>) {
    while let Some(first) = rx.recv().await {
        let mut bytes = first.size;
        let mut batch = vec![first];
        let deadline = tokio::time::Instant::now() + cfg.linger;
        while batch.len() < cfg.max_batch_records && bytes < cfg.max_batch_bytes {
            tokio::select! {
                job = rx.recv() => match job {
                    Some(j) => { bytes += j.size; batch.push(j); }
                    None => break,
                },
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        // Drop jobs whose caller already gave up (timeout) — they were never acknowledged.
        let (records, responders): (Vec<Record>, Vec<_>) =
            batch.into_iter().filter(|j| !j.respond.is_closed()).map(|j| (j.record, j.respond)).unzip();
        if records.is_empty() {
            continue;
        }
        let result = produce_with_retry(&pc, records, &cfg, &metrics).await;
        Metrics::inc(&metrics.publish_batches, 1);
        match result {
            Ok(offsets) => {
                for (tx, off) in responders.into_iter().zip(offsets) {
                    let _ = tx.send(Ok(off));
                }
            }
            Err(e) => {
                Metrics::inc(&metrics.publish_failures, 1);
                tracing::warn!(partition = pc.partition(), error = %e, "publish failed; clients receive a retryable error");
                for tx in responders {
                    let _ = tx.send(Err(e.clone()));
                }
            }
        }
    }
    tracing::info!(partition = pc.partition(), "publisher drained");
}

async fn produce_with_retry(
    pc: &PartitionClient,
    records: Vec<Record>,
    cfg: &PublisherConfig,
    metrics: &Metrics,
) -> Result<Vec<i64>, PublishError> {
    let mut last = String::new();
    for attempt in 0..cfg.max_attempts {
        if attempt > 0 {
            Metrics::inc(&metrics.publish_retries, 1);
            tokio::time::sleep(backoff(attempt, Duration::from_millis(100), Duration::from_secs(2))).await;
        }
        match tokio::time::timeout(cfg.attempt_timeout, pc.produce(records.clone(), Compression::NoCompression)).await {
            Ok(Ok(offsets)) if offsets.len() == records.len() => return Ok(offsets),
            Ok(Ok(offsets)) => {
                last = format!("broker returned {} offsets for {} records", offsets.len(), records.len())
            }
            Ok(Err(e)) => last = e.to_string(),
            Err(_) => last = "produce attempt timed out".into(),
        }
    }
    Err(PublishError::Unavailable(last))
}

/// In-process broker stand-in: partition workers answer jobs according to [`fake::Mode`]
/// instead of talking to Kafka.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Clone)]
    pub(crate) enum Mode {
        Ack,
        Fail(PublishError),
        /// Keep the job (and its responder) forever: the caller hits its ack timeout.
        Hang,
        /// Drop the responder without answering, as a crashed partition worker would.
        Drop,
    }

    #[derive(Debug, Clone)]
    pub(crate) struct FakeBroker {
        mode: Arc<Mutex<Mode>>,
        records: Arc<Mutex<Vec<(usize, Record)>>>,
    }

    impl FakeBroker {
        pub fn set_mode(&self, mode: Mode) {
            *self.mode.lock().unwrap() = mode;
        }

        /// Records the broker acknowledged, with the partition they landed on.
        pub fn records(&self) -> Vec<(usize, Record)> {
            self.records.lock().unwrap().clone()
        }
    }

    pub(crate) fn publisher(partitions: usize, cfg: PublisherConfig, metrics: Arc<Metrics>) -> (Publisher, FakeBroker) {
        let broker = FakeBroker { mode: Arc::new(Mutex::new(Mode::Ack)), records: Arc::new(Mutex::new(Vec::new())) };
        let mut queues = Vec::new();
        for p in 0..partitions {
            let (tx, mut rx) = mpsc::channel::<Job>(cfg.queue_capacity);
            queues.push(tx);
            let b = broker.clone();
            tokio::spawn(async move {
                let mut held = Vec::new();
                let mut offset = 0i64;
                while let Some(job) = rx.recv().await {
                    let mode = b.mode.lock().unwrap().clone();
                    match mode {
                        Mode::Ack => {
                            b.records.lock().unwrap().push((p, job.record));
                            let _ = job.respond.send(Ok(offset));
                            offset += 1;
                        }
                        Mode::Fail(e) => {
                            let _ = job.respond.send(Err(e));
                        }
                        Mode::Hang => held.push(job),
                        Mode::Drop => drop(job),
                    }
                }
            });
        }
        let bytes = Arc::new(Semaphore::new(cfg.max_queued_bytes));
        (Publisher { inner: Arc::new(Inner { queues, bytes, cfg, metrics }) }, broker)
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Mode, publisher};
    use super::*;
    use std::sync::atomic::Ordering;

    fn cfg() -> PublisherConfig {
        PublisherConfig { ack_timeout: Duration::from_millis(200), ..PublisherConfig::default() }
    }

    fn headers() -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([("schema_version".to_owned(), b"1".to_vec())])
    }

    #[test]
    fn default_config_is_bounded() {
        let c = PublisherConfig::default();
        assert!(c.queue_capacity > 0 && c.max_batch_records > 0 && c.max_attempts > 0);
        assert!(c.max_batch_bytes <= c.max_queued_bytes);
        assert!(c.ack_timeout >= c.attempt_timeout);
        // Byte permits are u32; the budget must fit.
        assert!(u32::try_from(c.max_queued_bytes).is_ok());
    }

    #[test]
    fn partition_for_is_stable_and_in_range() {
        for n in 1..16 {
            for key in ["t1/p1/dev", "t2/p1/dev", "", "ключ"] {
                let p = partition_for(key, n);
                assert!(p < n);
                assert_eq!(p, partition_for(key, n));
            }
        }
        assert_eq!(partition_for("anything", 0), 0);
        let used: std::collections::BTreeSet<_> = (0..200).map(|i| partition_for(&format!("t{i}/p/e"), 4)).collect();
        assert_eq!(used.len(), 4, "keys spread over all partitions");
    }

    #[tokio::test]
    async fn acked_publish_returns_offset_and_releases_queue_bytes() {
        let metrics = Arc::new(Metrics::default());
        let (p, broker) = publisher(1, cfg(), metrics.clone());
        assert_eq!(p.publish("t/p/e", b"abc".to_vec(), headers()).await.unwrap(), 0);
        assert_eq!(p.publish("t/p/e", b"def".to_vec(), headers()).await.unwrap(), 1);
        assert_eq!(metrics.queued_bytes.load(Ordering::Relaxed), 0);
        let recs = broker.records();
        assert_eq!(recs.len(), 2);
        let r = &recs[0].1;
        assert_eq!(r.key.as_deref(), Some(&b"t/p/e"[..]));
        assert_eq!(r.value.as_deref(), Some(&b"abc"[..]));
        assert_eq!(r.headers, headers());
    }

    #[tokio::test]
    async fn same_key_always_lands_on_the_same_partition() {
        let (p, broker) = publisher(4, cfg(), Arc::new(Metrics::default()));
        for i in 0..40 {
            let key = format!("t{}/p/e", i % 5);
            p.publish(&key, vec![1], BTreeMap::new()).await.unwrap();
        }
        let mut seen = std::collections::HashMap::new();
        for (partition, r) in broker.records() {
            let key = r.key.unwrap();
            assert_eq!(*seen.entry(key).or_insert(partition), partition);
        }
        assert_eq!(seen.len(), 5);
    }

    #[tokio::test]
    async fn broker_failure_is_an_error_never_success() {
        let metrics = Arc::new(Metrics::default());
        let (p, broker) = publisher(2, cfg(), metrics.clone());
        broker.set_mode(Mode::Fail(PublishError::Unavailable("leader not available".into())));
        let err = p.publish("k", vec![1; 10], BTreeMap::new()).await.unwrap_err();
        assert!(matches!(err, PublishError::Unavailable(ref m) if m.contains("leader")));
        assert!(broker.records().is_empty());
        assert_eq!(metrics.queued_bytes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn missing_ack_times_out() {
        let metrics = Arc::new(Metrics::default());
        let (p, broker) = publisher(1, cfg(), metrics.clone());
        broker.set_mode(Mode::Hang);
        assert!(matches!(p.publish("k", vec![1; 10], BTreeMap::new()).await, Err(PublishError::Timeout)));
        assert_eq!(metrics.queued_bytes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn dropped_responder_is_unavailable() {
        let (p, broker) = publisher(1, cfg(), Arc::new(Metrics::default()));
        broker.set_mode(Mode::Drop);
        let err = p.publish("k", vec![1], BTreeMap::new()).await.unwrap_err();
        assert!(matches!(err, PublishError::Unavailable(ref m) if m.contains("stopped")));
    }

    #[tokio::test]
    async fn full_queue_is_overloaded_not_buffered() {
        let metrics = Arc::new(Metrics::default());
        let (tx, _rx) = mpsc::channel::<Job>(1);
        let c = PublisherConfig { queue_capacity: 1, ..cfg() };
        let bytes = Arc::new(Semaphore::new(c.max_queued_bytes));
        let p = Publisher { inner: Arc::new(Inner { queues: vec![tx], bytes, cfg: c, metrics: metrics.clone() }) };
        let first = {
            let p = p.clone();
            tokio::spawn(async move { p.publish("k", vec![1], BTreeMap::new()).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(matches!(p.publish("k", vec![2], BTreeMap::new()).await, Err(PublishError::Overloaded)));
        assert!(matches!(first.await.unwrap(), Err(PublishError::Timeout)));
        assert_eq!(metrics.queued_bytes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn byte_budget_rejects_when_exhausted() {
        let c = PublisherConfig { max_queued_bytes: 16, ..cfg() };
        let (p, broker) = publisher(1, c, Arc::new(Metrics::default()));
        broker.set_mode(Mode::Hang);
        let first = {
            let p = p.clone();
            tokio::spawn(async move { p.publish("k", vec![0; 15], BTreeMap::new()).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(matches!(p.publish("k", vec![0; 4], BTreeMap::new()).await, Err(PublishError::Overloaded)));
        assert!(matches!(first.await.unwrap(), Err(PublishError::Timeout)));
        // Permits are released after the first request finished.
        broker.set_mode(Mode::Ack);
        assert!(p.publish("k", vec![0; 4], BTreeMap::new()).await.is_ok());
    }

    #[tokio::test]
    async fn oversized_record_is_capped_to_the_budget_not_rejected() {
        let c = PublisherConfig { max_queued_bytes: 8, ..cfg() };
        let (p, _broker) = publisher(1, c, Arc::new(Metrics::default()));
        assert!(p.publish("k", vec![0; 100], BTreeMap::new()).await.is_ok());
    }

    #[tokio::test]
    async fn no_partitions_is_unavailable_without_leaking_the_queue_gauge() {
        let metrics = Arc::new(Metrics::default());
        let (p, _broker) = publisher(0, cfg(), metrics.clone());
        let err = p.publish("k", vec![1; 32], BTreeMap::new()).await.unwrap_err();
        assert!(matches!(err, PublishError::Unavailable(ref m) if m.contains("no partitions")));
        assert_eq!(metrics.queued_bytes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn publish_errors_have_actionable_messages() {
        assert_eq!(PublishError::Overloaded.to_string(), "ingest queue is full");
        assert_eq!(PublishError::Timeout.to_string(), "timed out waiting for broker acknowledgement");
        assert_eq!(PublishError::Unavailable("x".into()).to_string(), "broker unavailable: x");
    }
}
