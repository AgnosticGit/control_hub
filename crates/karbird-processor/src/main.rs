//! karbird-processor: consumes ingest envelopes from the broker and writes
//! normalised rows to ClickHouse.
//!
//! Delivery: at-least-once. The partition offset (stored in PostgreSQL) is
//! advanced only after the batch has been written to ClickHouse and every
//! undecodable record has been recorded in `processing_errors`. A crash in
//! between leads to redelivery; ReplacingMergeTree keys make that idempotent
//! (eventually, see docs/architecture/data-flow.md).

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use clap::Parser;
use futures::{FutureExt, StreamExt, TryStreamExt};
use karbird_convert as convert;
#[cfg(test)]
use karbird_processor::batching::MAX_DEAD_LETTER_PAYLOAD;
use karbird_processor::batching::{dead_letter_payload, drain_ready_batch};
use karbird_protocol::{EDGE_CHANGES_TOPIC, EdgeChange, TELEMETRY_TOPIC};
use karbird_runtime::{backoff, connect_broker, ensure_topic, init_tracing, load_local_env, shutdown_signal};
use karbird_storage::ch::{ChConfig, ChStore};
use karbird_storage::pg::PgStore;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::Message;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::topic_partition_list::Offset;
use rdkafka::util::Timeout;
use tokio::sync::watch;

const CONSUMER: &str = "karbird-processor";
const MAX_BATCH_RECORDS: usize = 128;
const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONCURRENT_EDGE_PUBLISHES: usize = 32;
const IDLE_TOUCH_INTERVAL: Duration = Duration::from_secs(5);

async fn bounded_try_for_each<T, F, Fut>(items: Vec<T>, concurrency: usize, operation: F) -> anyhow::Result<()>
where
    F: FnMut(T) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    futures::stream::iter(items).map(operation).buffer_unordered(concurrency.max(1)).try_collect::<Vec<_>>().await?;
    Ok(())
}

#[derive(Debug, Default)]
struct PartitionWatermark {
    ready: AtomicBool,
    earliest: AtomicI64,
    high: AtomicI64,
}

#[derive(Debug, Parser)]
#[command(name = "karbird-processor")]
struct Args {
    #[arg(long, env = "KARBIRD_PROCESSOR_DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "KARBIRD_CLICKHOUSE_URL", default_value = "http://127.0.0.1:18123")]
    clickhouse_url: String,
    #[arg(long, env = "KARBIRD_CLICKHOUSE_DATABASE", default_value = "karbird")]
    clickhouse_database: String,
    #[arg(long, env = "KARBIRD_PROCESSOR_CLICKHOUSE_USER")]
    clickhouse_user: String,
    #[arg(long, env = "KARBIRD_PROCESSOR_CLICKHOUSE_PASSWORD")]
    clickhouse_password: String,
    #[arg(long, env = "KARBIRD_KAFKA_BROKERS", default_value = "127.0.0.1:19092")]
    brokers: String,
    #[arg(long, env = "KARBIRD_TOPIC_PARTITIONS", default_value_t = 4)]
    partitions: i32,
    #[arg(long, env = "KARBIRD_PROCESSOR_ADMIN_ADDR", default_value = "127.0.0.1:9465")]
    admin_addr: SocketAddr,
}

#[derive(Debug, Default)]
struct Metrics {
    records: AtomicU64,
    spans: AtomicU64,
    logs: AtomicU64,
    events: AtomicU64,
    metrics: AtomicU64,
    dropped_metric_points: AtomicU64,
    write_failures: AtomicU64,
    dead_letters: AtomicU64,
    offset_gaps: AtomicU64,
    lag: Vec<AtomicI64>,
    watermarks: Vec<PartitionWatermark>,
}

impl Metrics {
    fn render(&self) -> String {
        let mut s = format!(
            "karbird_processor_records_total {}\nkarbird_processor_rows_total{{table=\"spans\"}} {}\nkarbird_processor_rows_total{{table=\"logs\"}} {}\nkarbird_processor_rows_total{{table=\"events\"}} {}\nkarbird_processor_rows_total{{table=\"metrics\"}} {}\nkarbird_processor_dropped_metric_points_total {}\nkarbird_processor_write_failures_total {}\nkarbird_processor_dead_letters_total {}\nkarbird_processor_offset_gaps_total {}\n",
            self.records.load(Ordering::Relaxed),
            self.spans.load(Ordering::Relaxed),
            self.logs.load(Ordering::Relaxed),
            self.events.load(Ordering::Relaxed),
            self.metrics.load(Ordering::Relaxed),
            self.dropped_metric_points.load(Ordering::Relaxed),
            self.write_failures.load(Ordering::Relaxed),
            self.dead_letters.load(Ordering::Relaxed),
            self.offset_gaps.load(Ordering::Relaxed),
        );
        for (p, lag) in self.lag.iter().enumerate() {
            s.push_str(&format!("karbird_processor_lag{{partition=\"{p}\"}} {}\n", lag.load(Ordering::Relaxed)));
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
    edge_producer: FutureProducer,
}

fn edge_changes(
    facts: &[karbird_storage::ch::EdgeSpanFactRow],
    changed: &std::collections::HashSet<(String, String, String, String, String)>,
) -> Vec<EdgeChange> {
    use karbird_domain::{EntityId, EnvironmentId, ProjectId, Scope, TenantId};
    use std::collections::HashMap;

    type SpanKey = (String, String, String, String, String);
    let key = |fact: &karbird_storage::ch::EdgeSpanFactRow| -> SpanKey {
        (
            fact.tenant_id.clone(),
            fact.project_id.clone(),
            fact.environment_id.clone(),
            fact.trace_id.clone(),
            fact.span_id.clone(),
        )
    };
    let spans: HashMap<SpanKey, &karbird_storage::ch::EdgeSpanFactRow> =
        facts.iter().map(|fact| (key(fact), fact)).collect();
    let mut changes = Vec::new();
    for span in facts {
        let span_key = key(span);
        let current_span = changed.contains(&span_key);
        let mut add = |source: String,
                       target: String,
                       relation: &str,
                       request_bytes: Option<u64>,
                       response_bytes: Option<u64>,
                       cursor_ns: i64| {
            if source.is_empty() || target.is_empty() || source == target {
                return;
            }
            let id = format!("{}:{}:{relation}:{source}:{target}", span.trace_id, span.span_id);
            changes.push(EdgeChange {
                id,
                tenant_id: span.tenant_id.clone(),
                project_id: span.project_id.clone(),
                environment_id: span.environment_id.clone(),
                source,
                target,
                relation: relation.to_owned(),
                trace_id: span.trace_id.clone(),
                span_id: span.span_id.clone(),
                time_ns: span.start_time.to_string(),
                storage_cursor_ns: cursor_ns.to_string(),
                request_bytes,
                response_bytes,
            });
        };
        if current_span && matches!(span.kind, 3 | 4) && !span.target_entity_id.is_empty() {
            add(
                span.service_id.clone(),
                span.target_entity_id.clone(),
                "depends_on",
                span.request_size,
                span.response_size,
                span.ingested_at,
            );
        } else if matches!(span.kind, 2 | 5) && !span.parent_span_id.is_empty() {
            let parent_key = (
                span.tenant_id.clone(),
                span.project_id.clone(),
                span.environment_id.clone(),
                span.trace_id.clone(),
                span.parent_span_id.clone(),
            );
            if let Some(parent) = spans.get(&parent_key).filter(|_| current_span || changed.contains(&parent_key)) {
                add(
                    parent.service_id.clone(),
                    span.service_id.clone(),
                    "calls",
                    span.request_size.or(parent.request_size),
                    span.response_size.or(parent.response_size),
                    span.ingested_at.max(parent.ingested_at),
                );
            }
        } else if current_span
            && span.kind == 2
            && span.parent_span_id.is_empty()
            && let (Ok(tenant_id), Ok(project_id), Ok(environment_id)) = (
                TenantId::parse(&span.tenant_id),
                ProjectId::parse(&span.project_id),
                EnvironmentId::parse(&span.environment_id),
            )
        {
            let scope = Scope { tenant_id, project_id, environment_id };
            let clients = EntityId::derive_resource(&scope, "client", "external", "clients").to_string();
            add(clients, span.service_id.clone(), "calls", span.request_size, span.response_size, span.ingested_at);
        }
    }
    changes
}

async fn publish_edge_change(ctx: &Ctx, change: &EdgeChange) -> anyhow::Result<()> {
    let payload = serde_json::to_vec(change)?;
    let mut attempt = 0;
    loop {
        let record = FutureRecord::to(EDGE_CHANGES_TOPIC).key(&change.id).payload(&payload);
        match ctx.edge_producer.send(record, Duration::from_secs(10)).await {
            Ok(_) => return Ok(()),
            Err((error, _)) => {
                ctx.metrics.write_failures.fetch_add(1, Ordering::Relaxed);
                if *ctx.stop.borrow() {
                    anyhow::bail!("shutdown before edge change was acknowledged: {error}");
                }
                tracing::warn!(%error, %change.id, attempt, "edge change publish failed; source offset remains uncommitted");
                let mut stop = ctx.stop.clone();
                tokio::select! {
                    _ = tokio::time::sleep(backoff(attempt, Duration::from_millis(200), Duration::from_secs(10))) => {}
                    _ = stop.wait_for(|value| *value) => anyhow::bail!("shutdown before edge change was acknowledged"),
                }
                attempt += 1;
            }
        }
    }
}

fn durable_start_offset(saved: Option<i64>, earliest: i64) -> (i64, bool) {
    match saved {
        Some(offset) if offset < earliest => (earliest, true),
        Some(offset) => (offset, false),
        None => (earliest, false),
    }
}

/// Retry `op` with backoff until it succeeds or shutdown is requested.
async fn retry_until_ok<T, E: std::fmt::Display, F, Fut>(ctx: &Ctx, what: &str, op: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    retry_with(&ctx.metrics, &ctx.stop, what, op).await
}

async fn retry_with<T, E: std::fmt::Display, F, Fut>(
    metrics: &Metrics,
    stop_rx: &watch::Receiver<bool>,
    what: &str,
    mut op: F,
) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut attempt = 0;
    loop {
        match op().await {
            Ok(v) => return Some(v),
            Err(e) => {
                metrics.write_failures.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(error = %e, attempt, "{what} failed; will retry (offset not advanced)");
                if *stop_rx.borrow() {
                    return None;
                }
                let mut stop = stop_rx.clone();
                tokio::select! {
                    _ = tokio::time::sleep(backoff(attempt, Duration::from_millis(200), Duration::from_secs(10))) => {}
                    _ = stop.wait_for(|v| *v) => return None,
                }
                attempt += 1;
            }
        }
    }
}

async fn process_batch(
    ctx: &Ctx,
    consumer: &StreamConsumer,
    messages: &[rdkafka::message::OwnedMessage],
) -> anyhow::Result<()> {
    let Some(first) = messages.first() else { return Ok(()) };
    let Some(last) = messages.last() else { return Ok(()) };
    let partition = first.partition();
    if messages.iter().any(|message| message.partition() != partition) {
        anyhow::bail!("processor batch contains records from multiple partitions")
    }
    let next = last.offset().saturating_add(1);
    let high_watermark =
        ctx.metrics.watermarks.get(partition as usize).map_or(next, |watermark| watermark.high.load(Ordering::Relaxed));
    if let Some(lag) = ctx.metrics.lag.get(partition as usize) {
        lag.store(high_watermark.saturating_sub(next), Ordering::Relaxed);
    }
    let mut batch = convert::Converted::default();
    let mut dead = Vec::new();
    let mut max_received = 0_i64;
    for msg in messages {
        let value = msg.payload().unwrap_or_default();
        match convert::decode_envelope(value).and_then(|env| {
            max_received = max_received.max(env.received_time_unix_nano);
            convert::convert(&env, env.received_time_unix_nano)
        }) {
            Ok(converted) => batch.extend(converted),
            Err(error) => dead.push((msg.offset(), error, value.to_vec())),
        }
    }
    let (n_records, n_spans, n_logs, n_events, n_metrics, n_dropped) = (
        messages.len(),
        batch.spans.len(),
        batch.logs.len(),
        batch.events.len(),
        batch.metrics.len(),
        batch.dropped_points,
    );
    for (offset, error, value) in &dead {
        let payload = dead_letter_payload(value);
        let error_text = error.to_string();
        if retry_until_ok(ctx, "dead-letter write", || {
            ctx.pg.record_processing_error(
                CONSUMER,
                TELEMETRY_TOPIC,
                partition,
                *offset,
                None,
                None,
                error.kind(),
                &error_text,
                payload,
            )
        })
        .await
        .is_none()
        {
            anyhow::bail!("shutdown before dead-letter record was stored")
        }
        ctx.metrics.dead_letters.fetch_add(1, Ordering::Relaxed);
        tracing::error!(partition, broker_offset = offset, kind = error.kind(), "record moved to dead-letter table");
    }
    if !batch.is_empty()
        && retry_until_ok(ctx, "clickhouse insert", || async {
            ctx.ch.insert_spans(&batch.spans).await?;
            ctx.ch.insert_edge_span_facts(&batch.spans).await?;
            ctx.ch.insert_logs(&batch.logs).await?;
            ctx.ch.insert_events(&batch.events).await?;
            ctx.ch.insert_metrics(&batch.metrics).await?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .is_none()
    {
        anyhow::bail!("shutdown before ClickHouse accepted the record")
    }
    let mut changed_keys = std::collections::HashSet::new();
    let mut trace_ids_by_scope =
        std::collections::HashMap::<karbird_domain::Scope, std::collections::HashSet<String>>::new();
    for span in &batch.spans {
        changed_keys.insert((
            span.tenant_id.clone(),
            span.project_id.clone(),
            span.environment_id.clone(),
            span.trace_id.clone(),
            span.span_id.clone(),
        ));
        let scope = karbird_domain::Scope {
            tenant_id: karbird_domain::TenantId::parse(&span.tenant_id)?,
            project_id: karbird_domain::ProjectId::parse(&span.project_id)?,
            environment_id: karbird_domain::EnvironmentId::parse(&span.environment_id)?,
        };
        trace_ids_by_scope.entry(scope).or_default().insert(span.trace_id.clone());
    }
    let mut edge_facts = Vec::new();
    for (scope, trace_ids) in trace_ids_by_scope {
        let trace_ids = trace_ids.into_iter().collect::<Vec<_>>();
        let rows = retry_until_ok(ctx, "edge trace lookup", || ctx.ch.edge_facts_for_traces(&scope, &trace_ids))
            .await
            .ok_or_else(|| anyhow::anyhow!("shutdown before edge trace lookup completed"))?;
        edge_facts.extend(rows);
    }
    let edge_changes = edge_changes(&edge_facts, &changed_keys);
    bounded_try_for_each(edge_changes, MAX_CONCURRENT_EDGE_PUBLISHES, |change| async move {
        publish_edge_change(ctx, &change).await
    })
    .await?;
    // PostgreSQL remains authoritative: update it only after CH/dead-letter persistence,
    // then commit the Kafka group offset. If Kafka commit fails, replay is idempotent.
    if retry_until_ok(ctx, "offset commit", || {
        ctx.pg.commit_offset(CONSUMER, TELEMETRY_TOPIC, partition, next, high_watermark, max_received)
    })
    .await
    .is_none()
    {
        anyhow::bail!("shutdown before PostgreSQL offset was stored")
    }
    let mut offsets = rdkafka::TopicPartitionList::new();
    offsets.add_partition_offset(TELEMETRY_TOPIC, partition, Offset::Offset(next))?;
    if let Err(error) = consumer.commit(&offsets, CommitMode::Async) {
        // PostgreSQL is already durable; a later assignment seeks from that watermark.
        tracing::warn!(partition, next, %error, "Kafka group offset commit enqueue failed; replay is safe");
    }
    ctx.metrics.records.fetch_add(n_records as u64, Ordering::Relaxed);
    ctx.metrics.spans.fetch_add(n_spans as u64, Ordering::Relaxed);
    ctx.metrics.logs.fetch_add(n_logs as u64, Ordering::Relaxed);
    ctx.metrics.events.fetch_add(n_events as u64, Ordering::Relaxed);
    ctx.metrics.metrics.fetch_add(n_metrics as u64, Ordering::Relaxed);
    ctx.metrics.dropped_metric_points.fetch_add(n_dropped, Ordering::Relaxed);
    if n_dropped > 0 {
        tracing::warn!(partition, n_dropped, "metric points without a gauge/sum value were dropped");
    }
    tracing::debug!(partition, next, n_records, n_spans, n_logs, n_events, n_metrics, "batch stored");
    Ok(())
}

async fn run_consumer(config: &ClientConfig, partitions: i32, ctx: Ctx) -> anyhow::Result<()> {
    let consumer: StreamConsumer = config
        .clone()
        .set("group.id", CONSUMER)
        .set("enable.auto.commit", "false")
        .set("enable.auto.offset.store", "false")
        .set("auto.offset.reset", "earliest")
        .set("isolation.level", "read_committed")
        .create()?;
    consumer.subscribe(&[TELEMETRY_TOPIC])?;
    tracing::info!(partitions, "processor joined Kafka consumer group");
    let mut stop = ctx.stop.clone();
    let mut watermark_refresh = tokio::time::interval(IDLE_TOUCH_INTERVAL);
    watermark_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            message = consumer.recv() => match message {
                Ok(msg) => {
                    let messages = drain_ready_batch(
                        msg.detach(),
                        MAX_BATCH_RECORDS,
                        MAX_BATCH_BYTES,
                        |message| message.payload().map_or(0, <[u8]>::len),
                        || match consumer.recv().now_or_never() {
                            Some(Ok(message)) => Some(message.detach()),
                            Some(Err(error)) => {
                                tracing::warn!(%error, "Kafka consumer receive failed while draining ready messages");
                                None
                            }
                            None => None,
                        },
                    );
                    let mut by_partition: BTreeMap<i32, Vec<rdkafka::message::OwnedMessage>> = BTreeMap::new();
                    for message in messages {
                        by_partition.entry(message.partition()).or_default().push(message);
                    }
                    for (partition, batch) in by_partition {
                        let Some(watermark) = ctx.metrics.watermarks.get(partition as usize) else { continue; };
                        if !watermark.ready.load(Ordering::Acquire) {
                            let Some((low, high)) = retry_until_ok(&ctx, "initial Kafka watermarks", || async {
                                consumer.fetch_watermarks(TELEMETRY_TOPIC, partition, Timeout::After(Duration::from_secs(2)))
                            }).await else { break; };
                            watermark.earliest.store(low, Ordering::Relaxed);
                            watermark.high.store(high, Ordering::Relaxed);
                            watermark.ready.store(true, Ordering::Release);
                        }
                        let low = watermark.earliest.load(Ordering::Relaxed);
                        let saved = ctx.pg.load_offset(CONSUMER, TELEMETRY_TOPIC, partition).await?;
                        let saved_offset = saved.map(|offset| offset.next_offset);
                        let (expected, offset_gap) = durable_start_offset(saved_offset, low);
                        if offset_gap {
                            let old_offset = saved_offset.unwrap_or(low);
                            let error_text = format!("saved offset {old_offset} is before Kafka earliest {low}; telemetry records in between were lost");
                            ctx.metrics.offset_gaps.fetch_add(1, Ordering::Relaxed);
                            if retry_until_ok(&ctx, "offset gap record", || ctx.pg.record_processing_error(
                                CONSUMER, TELEMETRY_TOPIC, partition, old_offset,
                                None, None, "offset_out_of_range", &error_text, None,
                            )).await.is_none() { break; }
                            if retry_until_ok(&ctx, "offset gap watermark", || ctx.pg.commit_offset(
                                CONSUMER, TELEMETRY_TOPIC, partition, low, low, 0,
                            )).await.is_none() { break; }
                        }
                        if batch.first().is_some_and(|message| message.offset() != expected) {
                            if let Err(error) = consumer.seek(TELEMETRY_TOPIC, partition, Offset::Offset(expected), Timeout::After(Duration::from_secs(5))) {
                                tracing::debug!(partition, expected, %error, "partition assignment changed during offset seek; retrying on next delivery");
                            }
                            continue;
                        }
                        match process_batch(&ctx, &consumer, &batch).await {
                            Ok(()) => {}
                            Err(error) => {
                                ctx.metrics.write_failures.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!(partition, %error, "record batch failed; retrying from PostgreSQL watermark");
                                tokio::time::sleep(Duration::from_millis(500)).await;
                                let saved = ctx.pg.load_offset(CONSUMER, TELEMETRY_TOPIC, partition).await?;
                                let offset = saved.map_or(low, |saved| saved.next_offset);
                                if let Err(seek_error) = consumer.seek(TELEMETRY_TOPIC, partition, Offset::Offset(offset), Timeout::After(Duration::from_secs(5))) {
                                    tracing::debug!(partition, offset, error = %seek_error, "partition reassigned before retry seek; next assignment reloads PostgreSQL watermark");
                                }
                            }
                        }
                    }
                }
                Err(error) => { tracing::warn!(%error, "Kafka consumer receive failed"); tokio::time::sleep(Duration::from_millis(200)).await; }
            },
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() { break; }
            },
            _ = watermark_refresh.tick() => {
                for partition in 0..partitions {
                    let Some((low, high)) = retry_until_ok(&ctx, "Kafka watermarks", || async {
                        consumer.fetch_watermarks(TELEMETRY_TOPIC, partition, Timeout::After(Duration::from_secs(2)))
                    }).await else { break; };
                    if let Some(watermark) = ctx.metrics.watermarks.get(partition as usize) {
                        watermark.earliest.store(low, Ordering::Relaxed);
                        watermark.high.store(high, Ordering::Relaxed);
                        watermark.ready.store(true, Ordering::Release);
                    }
                    let saved = ctx.pg.load_offset(CONSUMER, TELEMETRY_TOPIC, partition).await?;
                    let next_offset = saved.map_or(low, |saved| saved.next_offset);
                    ctx.pg.commit_offset(CONSUMER, TELEMETRY_TOPIC, partition, next_offset, high, 0).await?;
                    if let Some(lag) = ctx.metrics.lag.get(partition as usize) {
                        lag.store(high.saturating_sub(next_offset), Ordering::Relaxed);
                    }
                }
            }
        }
    }
    consumer.unsubscribe();
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_local_env();
    init_tracing("karbird-processor");
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
    ensure_topic(&kafka, EDGE_CHANGES_TOPIC, args.partitions).await?;
    let edge_producer: FutureProducer = kafka
        .clone()
        .set("acks", "all")
        .set("enable.idempotence", "true")
        .set("message.timeout.ms", "10000")
        .create()?;

    let metrics = Arc::new(Metrics {
        lag: (0..partitions).map(|_| AtomicI64::new(0)).collect(),
        watermarks: (0..partitions).map(|_| PartitionWatermark::default()).collect(),
        ..Metrics::default()
    });
    let (stop_tx, stop_rx) = watch::channel(false);
    let ctx = Ctx { pg, ch, metrics: metrics.clone(), stop: stop_rx.clone(), edge_producer };
    let kafka_config = kafka;

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
    tracing::info!(partitions, admin = %args.admin_addr, "karbird-processor running");

    tokio::select! {
        _ = shutdown_signal() => {},
        result = run_consumer(&kafka_config, partitions, ctx) => {
            if let Err(error) = result { tracing::error!(%error, "processor worker stopped"); }
        }
    }
    let _ = stop_tx.send(true);
    let _ = admin_task.await;
    tracing::info!("karbird-processor stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    fn span(service: &str, id: &str, parent: &str, kind: u8, target: &str) -> karbird_storage::ch::SpanRow {
        karbird_storage::ch::SpanRow {
            tenant_id: "tenant-1".into(),
            project_id: "project-1".into(),
            environment_id: "prod".into(),
            service_id: service.into(),
            service_name: service.into(),
            service_namespace: String::new(),
            service_instance_id: String::new(),
            service_version: String::new(),
            trace_id: "trace-1".into(),
            span_id: id.into(),
            parent_span_id: parent.into(),
            name: "request".into(),
            kind,
            start_time: 1_790_000_000_000_000_000,
            end_time: 1_790_000_000_000_001_000,
            duration_ns: 1_000,
            status_code: 0,
            status_message: String::new(),
            target_entity_id: target.into(),
            target_name: String::new(),
            target_kind: String::new(),
            attributes: vec![("http.request.body.size".into(), "123".into())],
            resource_attributes: vec![],
            batch_id: "batch-1".into(),
            ingested_at: 1_790_000_000_000_002_000,
        }
    }

    #[test]
    fn edge_changes_are_stable_and_cover_calls_dependencies_and_external_clients() {
        let spans = [
            span("svc-a", "span-a", "", 2, ""),
            span("svc-b", "span-b", "span-a", 2, ""),
            span("svc-b", "span-c", "span-b", 3, "db-orders"),
        ];
        let facts = spans.iter().map(karbird_storage::ch::EdgeSpanFactRow::from).collect::<Vec<_>>();
        let changed = facts.iter().map(edge_fact_key).collect::<std::collections::HashSet<_>>();
        let changes = edge_changes(&facts, &changed);
        assert_eq!(changes.len(), 3);
        assert_eq!((changes[0].relation.as_str(), changes[0].target.as_str()), ("calls", "svc-a"));
        assert!(changes[0].source.starts_with("res_"));
        assert_eq!(
            (changes[1].relation.as_str(), changes[1].source.as_str(), changes[1].target.as_str()),
            ("calls", "svc-a", "svc-b")
        );
        assert_eq!(
            (changes[2].relation.as_str(), changes[2].source.as_str(), changes[2].target.as_str()),
            ("depends_on", "svc-b", "db-orders")
        );
        assert_eq!(
            changes,
            edge_changes(&facts, &changed),
            "replaying the source rows must yield identical IDs and values"
        );
        assert_eq!(changes[1].request_bytes, Some(123));
    }

    #[test]
    fn edge_calls_resolve_parent_and_child_arriving_in_separate_messages_in_either_order() {
        let mut child = span("svc-b", "child", "parent-client", 2, "");
        child.attributes.clear();
        let mut parent = span("svc-a", "parent-client", "", 1, "");
        parent.attributes =
            vec![("http.request.body.size".into(), "512".into()), ("http.response.body.size".into(), "2048".into())];
        let child_fact = karbird_storage::ch::EdgeSpanFactRow::from(&child);
        let parent_fact = karbird_storage::ch::EdgeSpanFactRow::from(&parent);
        let facts = [child_fact.clone(), parent_fact.clone()];

        // Parent arrives after child: publishing parent change must materialize the call.
        let changed_parent = [edge_fact_key(&parent_fact)].into_iter().collect();
        let changes = edge_changes(&facts, &changed_parent);

        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].source.as_str(), changes[0].target.as_str()), ("svc-a", "svc-b"));
        assert_eq!((changes[0].request_bytes, changes[0].response_bytes), (Some(512), Some(2048)));
        assert_eq!(changes[0].storage_cursor_ns, parent_fact.ingested_at.to_string());

        // Child arrives after parent: sizes fall back to parent facts.
        let changed_child = [edge_fact_key(&child_fact)].into_iter().collect();
        let changes = edge_changes(&facts, &changed_child);
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].request_bytes, changes[0].response_bytes), (Some(512), Some(2048)));
    }

    fn edge_fact_key(fact: &karbird_storage::ch::EdgeSpanFactRow) -> (String, String, String, String, String) {
        (
            fact.tenant_id.clone(),
            fact.project_id.clone(),
            fact.environment_id.clone(),
            fact.trace_id.clone(),
            fact.span_id.clone(),
        )
    }

    #[test]
    fn ready_messages_are_batched_with_record_and_byte_limits() {
        let mut queued = std::collections::VecDeque::from([4_usize, 5, 2]);
        let batch = drain_ready_batch(3, 8, 6, |size| *size, || queued.pop_front());
        assert_eq!(batch, [3, 4]);
        assert_eq!(queued, std::collections::VecDeque::from([5_usize, 2]));

        let mut queued = std::collections::VecDeque::from([1_usize, 1, 1]);
        let batch = drain_ready_batch(0, 2, 16, |_| 1, || queued.pop_front());
        assert_eq!(batch, [0, 1]);
        assert_eq!(queued, std::collections::VecDeque::from([1_usize, 1]));
    }

    #[tokio::test]
    async fn bounded_work_keeps_concurrency_within_limit() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let items = (0..12).collect::<Vec<_>>();
        bounded_try_for_each(items, 3, {
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            move |_| {
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(current, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok::<(), anyhow::Error>(())
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(max_active.load(Ordering::SeqCst), 3);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn dead_letter_payload_is_bounded() {
        assert_eq!(dead_letter_payload(b"abc"), Some(&b"abc"[..]));
        assert_eq!(dead_letter_payload(&[]), Some(&[][..]));
        let exact = vec![0u8; MAX_DEAD_LETTER_PAYLOAD];
        assert!(dead_letter_payload(&exact).is_some());
        let over = vec![0u8; MAX_DEAD_LETTER_PAYLOAD + 1];
        assert!(dead_letter_payload(&over).is_none());
    }

    #[test]
    fn pg_offset_is_authoritative_and_retention_gaps_are_detectable() {
        assert_eq!(durable_start_offset(None, 0), (0, false));
        assert_eq!(durable_start_offset(None, 17), (17, false));
        assert_eq!(durable_start_offset(Some(20), 17), (20, false));
        assert_eq!(durable_start_offset(Some(10), 17), (17, true));
    }

    #[test]
    fn metrics_render_every_series() {
        let m = Metrics { lag: (0..3).map(|_| AtomicI64::new(0)).collect(), ..Metrics::default() };
        m.records.fetch_add(10, Ordering::Relaxed);
        m.spans.fetch_add(4, Ordering::Relaxed);
        m.dead_letters.fetch_add(1, Ordering::Relaxed);
        m.offset_gaps.fetch_add(2, Ordering::Relaxed);
        m.metrics.fetch_add(6, Ordering::Relaxed);
        m.dropped_metric_points.fetch_add(1, Ordering::Relaxed);
        if let Some(l) = m.lag.get(2) {
            l.store(-3, Ordering::Relaxed);
        }
        let out = m.render();
        for line in [
            "karbird_processor_records_total 10",
            "karbird_processor_rows_total{table=\"spans\"} 4",
            "karbird_processor_rows_total{table=\"logs\"} 0",
            "karbird_processor_rows_total{table=\"events\"} 0",
            "karbird_processor_rows_total{table=\"metrics\"} 6",
            "karbird_processor_dropped_metric_points_total 1",
            "karbird_processor_write_failures_total 0",
            "karbird_processor_dead_letters_total 1",
            "karbird_processor_offset_gaps_total 2",
            "karbird_processor_lag{partition=\"0\"} 0",
            "karbird_processor_lag{partition=\"2\"} -3",
        ] {
            assert!(out.lines().any(|l| l == line), "missing {line}:\n{out}");
        }
        assert_eq!(out.lines().count(), 9 + 3);
        assert_eq!(Metrics::default().render().lines().count(), 9);
    }

    #[tokio::test]
    async fn retry_returns_immediately_on_success() {
        let m = Metrics::default();
        let (_tx, rx) = watch::channel(false);
        let r = retry_with(&m, &rx, "op", || async { Ok::<_, String>(5) }).await;
        assert_eq!(r, Some(5));
        assert_eq!(m.write_failures.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn retry_counts_failures_until_success() {
        let m = Metrics::default();
        let (_tx, rx) = watch::channel(false);
        let calls = AtomicU32::new(0);
        let r = retry_with(&m, &rx, "op", || {
            let n = calls.fetch_add(1, Ordering::Relaxed);
            async move { if n < 2 { Err("transient") } else { Ok(n) } }
        })
        .await;
        assert_eq!(r, Some(2));
        assert_eq!(m.write_failures.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn retry_gives_up_when_already_stopping() {
        let m = Metrics::default();
        let (_tx, rx) = watch::channel(true);
        let r: Option<()> = retry_with(&m, &rx, "op", || async { Err("down") }).await;
        assert_eq!(r, None, "offset must not advance");
        assert_eq!(m.write_failures.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn retry_is_interrupted_by_shutdown_during_backoff() {
        let m = Metrics::default();
        let (tx, rx) = watch::channel(false);
        let stopper = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx.send(true);
            tx
        });
        let started = Instant::now();
        let r: Option<()> = retry_with(&m, &rx, "op", || async { Err("down") }).await;
        assert_eq!(r, None);
        assert!(started.elapsed() < Duration::from_secs(5), "shutdown must cut the backoff short");
        assert!(m.write_failures.load(Ordering::Relaxed) >= 1);
        drop(stopper.await);
    }
}
