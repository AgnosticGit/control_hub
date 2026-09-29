//! controlhub-processor: consumes ingest envelopes from the broker and writes
//! normalised rows to ClickHouse.
//!
//! Delivery: at-least-once. The partition offset (stored in PostgreSQL) is
//! advanced only after the batch has been written to ClickHouse and every
//! undecodable record has been recorded in `processing_errors`. A crash in
//! between leads to redelivery; ReplacingMergeTree keys make that idempotent
//! (eventually, see docs/architecture/data-flow.md).

mod convert;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use clap::Parser;
use controlhub_protocol::TELEMETRY_TOPIC;
use controlhub_runtime::{backoff, connect_broker, ensure_topic, init_tracing, load_local_env, shutdown_signal};
use controlhub_storage::ch::{ChConfig, ChStore};
use controlhub_storage::pg::PgStore;
use rskafka::client::Client;
use rskafka::client::error::{Error as KafkaError, ProtocolError};
use rskafka::client::partition::{OffsetAt, PartitionClient, UnknownTopicHandling};
use tokio::sync::watch;

const CONSUMER: &str = "controlhub-processor";
const MAX_DEAD_LETTER_PAYLOAD: usize = 1024 * 1024;
const IDLE_TOUCH_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Parser)]
#[command(name = "controlhub-processor")]
struct Args {
    #[arg(long, env = "CONTROLHUB_PROCESSOR_DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "CONTROLHUB_CLICKHOUSE_URL", default_value = "http://127.0.0.1:18123")]
    clickhouse_url: String,
    #[arg(long, env = "CONTROLHUB_CLICKHOUSE_DATABASE", default_value = "controlhub")]
    clickhouse_database: String,
    #[arg(long, env = "CONTROLHUB_PROCESSOR_CLICKHOUSE_USER")]
    clickhouse_user: String,
    #[arg(long, env = "CONTROLHUB_PROCESSOR_CLICKHOUSE_PASSWORD")]
    clickhouse_password: String,
    #[arg(long, env = "CONTROLHUB_KAFKA_BROKERS", default_value = "127.0.0.1:19092")]
    brokers: String,
    #[arg(long, env = "CONTROLHUB_TOPIC_PARTITIONS", default_value_t = 4)]
    partitions: i32,
    #[arg(long, env = "CONTROLHUB_PROCESSOR_ADMIN_ADDR", default_value = "127.0.0.1:9465")]
    admin_addr: SocketAddr,
}

#[derive(Debug, Default)]
struct Metrics {
    records: AtomicU64,
    spans: AtomicU64,
    logs: AtomicU64,
    events: AtomicU64,
    write_failures: AtomicU64,
    dead_letters: AtomicU64,
    offset_gaps: AtomicU64,
    lag: Vec<AtomicI64>,
}

impl Metrics {
    fn render(&self) -> String {
        let mut s = format!(
            "controlhub_processor_records_total {}\ncontrolhub_processor_rows_total{{table=\"spans\"}} {}\ncontrolhub_processor_rows_total{{table=\"logs\"}} {}\ncontrolhub_processor_rows_total{{table=\"events\"}} {}\ncontrolhub_processor_write_failures_total {}\ncontrolhub_processor_dead_letters_total {}\ncontrolhub_processor_offset_gaps_total {}\n",
            self.records.load(Ordering::Relaxed),
            self.spans.load(Ordering::Relaxed),
            self.logs.load(Ordering::Relaxed),
            self.events.load(Ordering::Relaxed),
            self.write_failures.load(Ordering::Relaxed),
            self.dead_letters.load(Ordering::Relaxed),
            self.offset_gaps.load(Ordering::Relaxed),
        );
        for (p, lag) in self.lag.iter().enumerate() {
            s.push_str(&format!("controlhub_processor_lag{{partition=\"{p}\"}} {}\n", lag.load(Ordering::Relaxed)));
        }
        s
    }
}

#[derive(Clone)]
struct Ctx {
    pg: PgStore,
    ch: ChStore,
    metrics: Arc<Metrics>,
    stop: watch::Receiver<bool>,
}

fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
}

/// Retry `op` with backoff until it succeeds or shutdown is requested.
async fn retry_until_ok<T, E: std::fmt::Display, F, Fut>(ctx: &Ctx, what: &str, mut op: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut attempt = 0;
    loop {
        match op().await {
            Ok(v) => return Some(v),
            Err(e) => {
                ctx.metrics.write_failures.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(error = %e, attempt, "{what} failed; will retry (offset not advanced)");
                if *ctx.stop.borrow() {
                    return None;
                }
                let mut stop = ctx.stop.clone();
                tokio::select! {
                    _ = tokio::time::sleep(backoff(attempt, Duration::from_millis(200), Duration::from_secs(10))) => {}
                    _ = stop.wait_for(|v| *v) => return None,
                }
                attempt += 1;
            }
        }
    }
}

async fn run_partition(ctx: Ctx, pc: PartitionClient) {
    let partition = pc.partition();
    let mut stop = ctx.stop.clone();
    let mut offset =
        match retry_until_ok(&ctx, "load offset", || ctx.pg.load_offset(CONSUMER, TELEMETRY_TOPIC, partition)).await {
            Some(Some(o)) => o.next_offset,
            Some(None) => match pc.get_offset(OffsetAt::Earliest).await {
                Ok(o) => o,
                Err(e) => {
                    tracing::error!(partition, error = %e, "cannot determine earliest offset");
                    0
                }
            },
            None => return,
        };
    tracing::info!(partition, offset, "partition consumer started");
    let mut idle_touched: Option<Instant> = None;

    loop {
        if *stop.borrow() {
            break;
        }
        let fetched = tokio::select! {
            r = pc.fetch_records(offset, 1..8_000_000, 500) => r,
            _ = stop.wait_for(|v| *v) => break,
        };
        let (records, high_watermark) = match fetched {
            Ok(v) => v,
            Err(KafkaError::ServerError { protocol_error: ProtocolError::OffsetOutOfRange, .. }) => {
                // Data expired from the broker before we processed it: make the gap visible.
                let earliest = pc.get_offset(OffsetAt::Earliest).await.unwrap_or(offset);
                let msg = format!(
                    "offset {offset} out of range; resuming at {earliest}. Records in between were not processed."
                );
                tracing::error!(partition, "{msg}");
                ctx.metrics.offset_gaps.fetch_add(1, Ordering::Relaxed);
                let _ = ctx
                    .pg
                    .record_processing_error(
                        CONSUMER,
                        TELEMETRY_TOPIC,
                        partition,
                        offset,
                        None,
                        None,
                        "offset_out_of_range",
                        &msg,
                        None,
                    )
                    .await;
                offset = earliest.max(0);
                continue;
            }
            Err(e) => {
                tracing::warn!(partition, error = %e, "fetch failed; retrying");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if let Some(lag) = ctx.metrics.lag.get(partition as usize) {
            lag.store(high_watermark - offset, Ordering::Relaxed);
        }
        if records.is_empty() {
            // An idle, caught-up partition must still advance the completeness watermark,
            // otherwise the API reports data as stale while nothing is pending.
            if offset >= high_watermark && idle_touched.is_none_or(|t| t.elapsed() >= IDLE_TOUCH_INTERVAL) {
                if let Err(e) =
                    ctx.pg.commit_offset(CONSUMER, TELEMETRY_TOPIC, partition, offset, high_watermark, 0).await
                {
                    tracing::warn!(partition, error = %e, "idle watermark update failed");
                }
                idle_touched = Some(Instant::now());
            }
            continue;
        }
        idle_touched = None;

        let ingested_at = now_ns();
        let mut batch = convert::Converted::default();
        let mut dead = Vec::new();
        let mut max_received = 0i64;
        for r in &records {
            let value = r.record.value.as_deref().unwrap_or(&[]);
            match convert::decode_envelope(value).and_then(|env| {
                max_received = max_received.max(env.received_time_unix_nano);
                convert::convert(&env, ingested_at)
            }) {
                Ok(c) => batch.extend(c),
                Err(e) => dead.push((r.offset, e, value.to_vec())),
            }
        }

        let (n_spans, n_logs, n_events) = (batch.spans.len(), batch.logs.len(), batch.events.len());
        if !batch.is_empty() {
            let ok = retry_until_ok(&ctx, "clickhouse insert", || async {
                ctx.ch.insert_spans(&batch.spans).await?;
                ctx.ch.insert_logs(&batch.logs).await?;
                ctx.ch.insert_events(&batch.events).await
            })
            .await;
            if ok.is_none() {
                break; // shutdown before the batch was stored: offset NOT advanced
            }
        }
        for (off, err, payload) in &dead {
            let payload = (payload.len() <= MAX_DEAD_LETTER_PAYLOAD).then_some(payload.as_slice());
            let msg = err.to_string();
            let stored = retry_until_ok(&ctx, "dead-letter write", || {
                ctx.pg.record_processing_error(
                    CONSUMER,
                    TELEMETRY_TOPIC,
                    partition,
                    *off,
                    None,
                    None,
                    err.kind(),
                    &msg,
                    payload,
                )
            })
            .await;
            if stored.is_none() {
                return;
            }
            ctx.metrics.dead_letters.fetch_add(1, Ordering::Relaxed);
            tracing::error!(partition, broker_offset = off, kind = err.kind(), "record moved to dead-letter table");
        }
        let next = records.iter().map(|r| r.offset).max().unwrap_or(offset - 1) + 1;
        if retry_until_ok(&ctx, "offset commit", || {
            ctx.pg.commit_offset(CONSUMER, TELEMETRY_TOPIC, partition, next, high_watermark, max_received)
        })
        .await
        .is_none()
        {
            break;
        }
        offset = next;
        ctx.metrics.records.fetch_add(records.len() as u64, Ordering::Relaxed);
        ctx.metrics.spans.fetch_add(n_spans as u64, Ordering::Relaxed);
        ctx.metrics.logs.fetch_add(n_logs as u64, Ordering::Relaxed);
        ctx.metrics.events.fetch_add(n_events as u64, Ordering::Relaxed);
        tracing::debug!(partition, next, n_spans, n_logs, n_events, "batch stored");
    }
    tracing::info!(partition, offset, "partition consumer stopped");
}

async fn start(client: Arc<Client>, partitions: i32, ctx: Ctx) -> anyhow::Result<Vec<tokio::task::JoinHandle<()>>> {
    let mut handles = Vec::new();
    for p in 0..partitions {
        let pc = client.partition_client(TELEMETRY_TOPIC.to_owned(), p, UnknownTopicHandling::Retry).await?;
        handles.push(tokio::spawn(run_partition(ctx.clone(), pc)));
    }
    Ok(handles)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_local_env();
    init_tracing("controlhub-processor");
    let args = Args::parse();

    let pg = PgStore::connect(&args.database_url, 8).await?;
    let ch = ChStore::new(&ChConfig {
        url: args.clickhouse_url,
        database: args.clickhouse_database,
        user: args.clickhouse_user,
        password: args.clickhouse_password,
    });
    ch.ping().await?;
    let kafka = connect_broker(&args.brokers, Duration::from_secs(60)).await?;
    let partitions = ensure_topic(&kafka, TELEMETRY_TOPIC, args.partitions).await?;

    let metrics = Arc::new(Metrics { lag: (0..partitions).map(|_| AtomicI64::new(0)).collect(), ..Metrics::default() });
    let (stop_tx, stop_rx) = watch::channel(false);
    let ctx = Ctx { pg, ch, metrics: metrics.clone(), stop: stop_rx.clone() };
    let handles = start(kafka, partitions, ctx).await?;

    let m = metrics.clone();
    let admin = axum::Router::new().route(
        "/metrics",
        axum::routing::get(move || {
            let m = m.clone();
            async move { m.render() }
        }),
    );
    let listener = tokio::net::TcpListener::bind(args.admin_addr).await?;
    let mut admin_stop = stop_rx.clone();
    let admin_task = tokio::spawn(
        axum::serve(listener, admin)
            .with_graceful_shutdown(async move {
                let _ = admin_stop.wait_for(|v| *v).await;
            })
            .into_future(),
    );
    tracing::info!(partitions, admin = %args.admin_addr, "controlhub-processor running");

    shutdown_signal().await;
    let _ = stop_tx.send(true);
    for h in handles {
        let _ = tokio::time::timeout(Duration::from_secs(30), h).await;
    }
    let _ = admin_task.await;
    tracing::info!("controlhub-processor stopped");
    Ok(())
}
