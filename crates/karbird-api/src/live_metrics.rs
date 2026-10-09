//! Low-cardinality counters and gauges for the API's Kafka-to-WebSocket live path.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::live_hub::LiveHub;

#[derive(Debug, Default)]
pub struct LiveMetrics {
    pub records_total: AtomicU64,
    pub decode_errors_total: AtomicU64,
    pub items_published_total: AtomicU64,
    pub publish_retries_total: AtomicU64,
    pub offset_commit_enqueue_errors_total: AtomicU64,
    pub consumer_lag_messages: AtomicU64,
    pub lag_refresh_errors_total: AtomicU64,
}

impl LiveMetrics {
    pub fn render_prometheus(&self, hub: &LiveHub) -> String {
        let hub = hub.metrics();
        format!(
            concat!(
                "# HELP karbird_live_kafka_records_total Kafka records read by this API replica.\n",
                "# TYPE karbird_live_kafka_records_total counter\n",
                "karbird_live_kafka_records_total {}\n",
                "# HELP karbird_live_kafka_decode_errors_total Invalid Kafka envelopes skipped by the live reader.\n",
                "# TYPE karbird_live_kafka_decode_errors_total counter\n",
                "karbird_live_kafka_decode_errors_total {}\n",
                "# HELP karbird_live_items_published_total Timeline items admitted to the local fan-out hub.\n",
                "# TYPE karbird_live_items_published_total counter\n",
                "karbird_live_items_published_total {}\n",
                "# HELP karbird_live_publish_retries_total Hub backpressure retries before admission.\n",
                "# TYPE karbird_live_publish_retries_total counter\n",
                "karbird_live_publish_retries_total {}\n",
                "# HELP karbird_live_offset_commit_enqueue_errors_total Immediate errors while enqueuing Kafka offset commits.\n",
                "# TYPE karbird_live_offset_commit_enqueue_errors_total counter\n",
                "karbird_live_offset_commit_enqueue_errors_total {}\n",
                "# HELP karbird_live_consumer_lag_messages Sum of unconsumed records across assigned Kafka partitions.\n",
                "# TYPE karbird_live_consumer_lag_messages gauge\n",
                "karbird_live_consumer_lag_messages {}\n",
                "# HELP karbird_live_lag_refresh_errors_total Failed Kafka lag refreshes.\n",
                "# TYPE karbird_live_lag_refresh_errors_total counter\n",
                "karbird_live_lag_refresh_errors_total {}\n",
                "# HELP karbird_live_hub_subscriber_overflows_total Slow WebSocket queues closed after reaching capacity.\n",
                "# TYPE karbird_live_hub_subscriber_overflows_total counter\n",
                "karbird_live_hub_subscriber_overflows_total {}\n",
                "# HELP karbird_live_hub_scope_evictions_total Scope histories evicted from the bounded local hub.\n",
                "# TYPE karbird_live_hub_scope_evictions_total counter\n",
                "karbird_live_hub_scope_evictions_total {}\n",
                "# HELP karbird_live_hub_capacity_rejections_total Hub subscriptions rejected at active scope capacity.\n",
                "# TYPE karbird_live_hub_capacity_rejections_total counter\n",
                "karbird_live_hub_capacity_rejections_total {}\n"
            ),
            self.records_total.load(Ordering::Relaxed),
            self.decode_errors_total.load(Ordering::Relaxed),
            self.items_published_total.load(Ordering::Relaxed),
            self.publish_retries_total.load(Ordering::Relaxed),
            self.offset_commit_enqueue_errors_total.load(Ordering::Relaxed),
            self.consumer_lag_messages.load(Ordering::Relaxed),
            self.lag_refresh_errors_total.load(Ordering::Relaxed),
            hub.subscriber_overflows,
            hub.scope_evictions,
            hub.capacity_rejections,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_output_is_low_cardinality_and_prometheus_compatible() {
        let metrics = LiveMetrics::default();
        metrics.records_total.store(12, Ordering::Relaxed);
        metrics.consumer_lag_messages.store(3, Ordering::Relaxed);
        let output = metrics.render_prometheus(&LiveHub::default());
        assert!(output.contains("# TYPE karbird_live_kafka_records_total counter\n"));
        assert!(output.contains("karbird_live_kafka_records_total 12\n"));
        assert!(output.contains("karbird_live_consumer_lag_messages 3\n"));
        assert!(output.contains("karbird_live_offset_commit_enqueue_errors_total 0\n"));
        assert!(!output.contains("tenant"));
        assert!(!output.contains("project"));
        assert!(!output.contains("environment"));
    }
}
