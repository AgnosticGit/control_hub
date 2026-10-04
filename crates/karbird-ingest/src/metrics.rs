//! Ingest self-metrics (Prometheus text format on the loopback admin port).

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Metrics {
    pub requests_accepted: AtomicU64,
    pub requests_rejected_auth: AtomicU64,
    pub requests_rejected_invalid: AtomicU64,
    pub requests_rejected_overload: AtomicU64,
    pub requests_failed_broker: AtomicU64,
    pub spans_accepted: AtomicU64,
    pub spans_rejected: AtomicU64,
    pub logs_accepted: AtomicU64,
    pub logs_rejected: AtomicU64,
    pub events_accepted: AtomicU64,
    pub events_rejected: AtomicU64,
    pub metric_points_accepted: AtomicU64,
    pub metric_points_rejected: AtomicU64,
    pub metric_series_rejected: AtomicU64,
    pub attributes_redacted: AtomicU64,
    pub attributes_truncated: AtomicU64,
    pub publish_batches: AtomicU64,
    pub publish_retries: AtomicU64,
    pub publish_failures: AtomicU64,
    pub queued_bytes: AtomicU64,
}

impl Metrics {
    pub fn inc(c: &AtomicU64, v: u64) {
        c.fetch_add(v, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let rows: [(&str, &str, &AtomicU64); 20] = [
            ("karbird_ingest_requests_total", "result=\"accepted\"", &self.requests_accepted),
            ("karbird_ingest_requests_total", "result=\"rejected_auth\"", &self.requests_rejected_auth),
            ("karbird_ingest_requests_total", "result=\"rejected_invalid\"", &self.requests_rejected_invalid),
            ("karbird_ingest_requests_total", "result=\"rejected_overload\"", &self.requests_rejected_overload),
            ("karbird_ingest_requests_total", "result=\"failed_broker\"", &self.requests_failed_broker),
            ("karbird_ingest_items_total", "signal=\"spans\",result=\"accepted\"", &self.spans_accepted),
            ("karbird_ingest_items_total", "signal=\"spans\",result=\"rejected\"", &self.spans_rejected),
            ("karbird_ingest_items_total", "signal=\"logs\",result=\"accepted\"", &self.logs_accepted),
            ("karbird_ingest_items_total", "signal=\"logs\",result=\"rejected\"", &self.logs_rejected),
            ("karbird_ingest_items_total", "signal=\"events\",result=\"accepted\"", &self.events_accepted),
            ("karbird_ingest_items_total", "signal=\"events\",result=\"rejected\"", &self.events_rejected),
            ("karbird_ingest_items_total", "signal=\"metrics\",result=\"accepted\"", &self.metric_points_accepted),
            ("karbird_ingest_items_total", "signal=\"metrics\",result=\"rejected\"", &self.metric_points_rejected),
            ("karbird_ingest_metric_points_series_limited_total", "", &self.metric_series_rejected),
            ("karbird_ingest_attributes_redacted_total", "", &self.attributes_redacted),
            ("karbird_ingest_attributes_truncated_total", "", &self.attributes_truncated),
            ("karbird_ingest_publish_batches_total", "", &self.publish_batches),
            ("karbird_ingest_publish_retries_total", "", &self.publish_retries),
            ("karbird_ingest_publish_failures_total", "", &self.publish_failures),
            ("karbird_ingest_queued_bytes", "", &self.queued_bytes),
        ];
        for (name, labels, v) in rows {
            let v = v.load(Ordering::Relaxed);
            if labels.is_empty() {
                let _ = writeln!(out, "{name} {v}");
            } else {
                let _ = writeln!(out, "{name}{{{labels}}} {v}");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_every_counter_with_zero_defaults() {
        let out = Metrics::default().render();
        assert_eq!(out.lines().count(), 20);
        assert!(out.lines().all(|l| l.ends_with(" 0")), "{out}");
        assert!(out.contains("karbird_ingest_requests_total{result=\"failed_broker\"} 0\n"));
        assert!(out.contains("karbird_ingest_attributes_truncated_total 0\n"));
    }

    #[test]
    fn renders_incremented_values_under_the_right_labels() {
        let m = Metrics::default();
        Metrics::inc(&m.requests_accepted, 3);
        Metrics::inc(&m.requests_accepted, 2);
        Metrics::inc(&m.spans_rejected, 7);
        Metrics::inc(&m.events_rejected, 1);
        Metrics::inc(&m.queued_bytes, 42);
        Metrics::inc(&m.metric_points_rejected, 4);
        Metrics::inc(&m.metric_series_rejected, 2);
        let out = m.render();
        assert!(out.contains("karbird_ingest_items_total{signal=\"metrics\",result=\"rejected\"} 4\n"));
        assert!(out.contains("karbird_ingest_metric_points_series_limited_total 2\n"));
        assert!(out.contains("karbird_ingest_requests_total{result=\"accepted\"} 5\n"));
        assert!(out.contains("karbird_ingest_items_total{signal=\"spans\",result=\"rejected\"} 7\n"));
        assert!(out.contains("karbird_ingest_items_total{signal=\"events\",result=\"rejected\"} 1\n"));
        assert!(out.contains("karbird_ingest_items_total{signal=\"spans\",result=\"accepted\"} 0\n"));
        assert!(out.contains("karbird_ingest_queued_bytes 42\n"));
    }

    #[test]
    fn every_series_is_unique() {
        let out = Metrics::default().render();
        let series: std::collections::BTreeSet<_> =
            out.lines().filter_map(|l| l.rsplit_once(' ')).map(|(s, _)| s).collect();
        assert_eq!(series.len(), out.lines().count());
    }
}
