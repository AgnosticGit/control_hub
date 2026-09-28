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
        Metrics::inc(&inner.metrics.queued_bytes, size as u64);

        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        let idx = (h.finish() % inner.queues.len().max(1) as u64) as usize;
        let queue = inner.queues.get(idx).ok_or_else(|| PublishError::Unavailable("no partitions".into()))?;

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
