//! Kafka consumer that converts accepted envelopes into scoped local WebSocket fan-out.

use std::sync::Arc;
use std::time::Duration;

use rdkafka::Offset;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::Message;
use rdkafka::util::Timeout;

use crate::live_hub::{LiveHub, LiveItem};
use crate::live_metrics::LiveMetrics;

pub async fn run(consumer: StreamConsumer, live_hub: LiveHub, metrics: Arc<LiveMetrics>) {
    let consumer = Arc::new(consumer);
    let mut last_live_cursor = 0_i64;
    let mut lag_refresh = tokio::time::interval(Duration::from_secs(5));
    lag_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let received = tokio::select! {
            _ = lag_refresh.tick() => {
                let lag_consumer = Arc::clone(&consumer);
                match tokio::task::spawn_blocking(move || consumer_lag(&lag_consumer)).await {
                    Ok(Ok(lag)) => metrics.consumer_lag_messages.store(lag, std::sync::atomic::Ordering::Relaxed),
                    Ok(Err(error)) => {
                        metrics.lag_refresh_errors_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::debug!(%error, "live Kafka lag refresh failed");
                    }
                    Err(error) => {
                        metrics.lag_refresh_errors_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::debug!(%error, "live Kafka lag refresh task failed");
                    }
                }
                continue;
            }
            received = consumer.recv() => received,
        };
        match received {
            Ok(message) => {
                metrics.records_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(payload) = message.payload() else {
                    if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
                        metrics.offset_commit_enqueue_errors_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::warn!(%error, "commit empty live record");
                    }
                    continue;
                };
                let result = karbird_convert::decode_envelope(payload).and_then(|envelope| {
                    let scope = karbird_convert::scope_of(&envelope)?;
                    let converted =
                        karbird_convert::convert(&envelope, chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0))?;
                    Ok((scope, converted))
                });
                match result {
                    Ok((scope, converted)) => {
                        let clients_id = crate::routes::telemetry::external_clients_id(&scope);
                        let mut items =
                            Vec::with_capacity(converted.spans.len() + converted.logs.len() + converted.events.len());
                        for row in converted.spans {
                            let cursor_ns = row.ingested_at;
                            let item = crate::routes::telemetry::span_item(
                                karbird_storage::ch::TimelineSpanRow {
                                    trace_id: row.trace_id,
                                    span_id: row.span_id,
                                    parent_span_id: row.parent_span_id,
                                    service_id: row.service_id,
                                    service_name: row.service_name,
                                    target_entity_id: row.target_entity_id,
                                    name: row.name,
                                    kind: row.kind,
                                    start_time: row.start_time,
                                    end_time: row.end_time,
                                    status_code: row.status_code,
                                    ingested_at: row.ingested_at,
                                },
                                &clients_id,
                            );
                            items.push(LiveItem { cursor_ns, item });
                        }
                        for row in converted.logs {
                            let cursor_ns = row.ingested_at;
                            let item = crate::routes::telemetry::log_item(karbird_storage::ch::TimelineLogRow {
                                log_id: row.log_id,
                                service_id: row.service_id,
                                service_name: row.service_name,
                                time: row.time,
                                severity_number: row.severity_number,
                                trace_id: row.trace_id,
                                ingested_at: row.ingested_at,
                            });
                            items.push(LiveItem { cursor_ns, item });
                        }
                        for row in converted.events {
                            let cursor_ns = row.ingested_at;
                            items.push(LiveItem { cursor_ns, item: crate::routes::telemetry::event_item(row) });
                        }
                        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
                        let item_count = items.len() as u64;
                        for chunk in items.chunks(64) {
                            let cursor_ns = crate::advance_live_cursor(&mut last_live_cursor, now);
                            let batch: Vec<_> = chunk
                                .iter()
                                .cloned()
                                .map(|mut item| {
                                    item.cursor_ns = cursor_ns;
                                    item
                                })
                                .collect();
                            loop {
                                match live_hub.publish_batch(scope.clone(), batch.clone()) {
                                    Ok(()) => break,
                                    Err(error) => {
                                        metrics
                                            .publish_retries_total
                                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        tracing::warn!(%error, "live hub is full; applying backpressure to Kafka consumer");
                                        tokio::time::sleep(Duration::from_millis(100)).await;
                                    }
                                }
                            }
                        }
                        metrics.items_published_total.fetch_add(item_count, std::sync::atomic::Ordering::Relaxed);
                        if let Err(error) = consumer.commit_message(&message, CommitMode::Async) {
                            metrics
                                .offset_commit_enqueue_errors_total
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            tracing::warn!(%error, "live consumer offset commit failed; record may be replayed");
                        }
                    }
                    Err(error) => {
                        // The processor durably records malformed envelopes. The live reader
                        // cannot render one, so report and advance to avoid blocking the stream.
                        metrics.decode_errors_total.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::error!(%error, "live consumer skipped invalid envelope");
                        if let Err(commit_error) = consumer.commit_message(&message, CommitMode::Async) {
                            metrics
                                .offset_commit_enqueue_errors_total
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            tracing::warn!(error = %commit_error, "invalid live envelope offset commit failed");
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "live Kafka consumer receive failed");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

fn consumer_lag(consumer: &StreamConsumer) -> rdkafka::error::KafkaResult<u64> {
    let assignment = consumer.assignment()?;
    let position = consumer.position()?;
    let mut total = 0_u64;
    for partition in assignment.elements() {
        let topic = partition.topic();
        let id = partition.partition();
        let (low, high) = consumer.fetch_watermarks(topic, id, Timeout::After(Duration::from_secs(2)))?;
        let next_offset = position
            .find_partition(topic, id)
            .map(|position| match position.offset() {
                Offset::Offset(offset) => offset,
                Offset::End => high,
                _ => low,
            })
            .unwrap_or(low);
        total = total.saturating_add(high.saturating_sub(next_offset).max(0) as u64);
    }
    Ok(total)
}
