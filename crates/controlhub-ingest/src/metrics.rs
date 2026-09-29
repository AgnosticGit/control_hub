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
        let rows: [(&str, &str, &AtomicU64); 17] = [
            ("controlhub_ingest_requests_total", "result=\"accepted\"", &self.requests_accepted),
            ("controlhub_ingest_requests_total", "result=\"rejected_auth\"", &self.requests_rejected_auth),
            ("controlhub_ingest_requests_total", "result=\"rejected_invalid\"", &self.requests_rejected_invalid),
            ("controlhub_ingest_requests_total", "result=\"rejected_overload\"", &self.requests_rejected_overload),
            ("controlhub_ingest_requests_total", "result=\"failed_broker\"", &self.requests_failed_broker),
            ("controlhub_ingest_items_total", "signal=\"spans\",result=\"accepted\"", &self.spans_accepted),
            ("controlhub_ingest_items_total", "signal=\"spans\",result=\"rejected\"", &self.spans_rejected),
            ("controlhub_ingest_items_total", "signal=\"logs\",result=\"accepted\"", &self.logs_accepted),
            ("controlhub_ingest_items_total", "signal=\"logs\",result=\"rejected\"", &self.logs_rejected),
            ("controlhub_ingest_items_total", "signal=\"events\",result=\"accepted\"", &self.events_accepted),
            ("controlhub_ingest_items_total", "signal=\"events\",result=\"rejected\"", &self.events_rejected),
            ("controlhub_ingest_attributes_redacted_total", "", &self.attributes_redacted),
            ("controlhub_ingest_attributes_truncated_total", "", &self.attributes_truncated),
            ("controlhub_ingest_publish_batches_total", "", &self.publish_batches),
            ("controlhub_ingest_publish_retries_total", "", &self.publish_retries),
            ("controlhub_ingest_publish_failures_total", "", &self.publish_failures),
            ("controlhub_ingest_queued_bytes", "", &self.queued_bytes),
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
        assert_eq!(out.lines().count(), 17);
        assert!(out.lines().all(|l| l.ends_with(" 0")), "{out}");
        assert!(out.contains("controlhub_ingest_requests_total{result=\"failed_broker\"} 0\n"));
        assert!(out.contains("controlhub_ingest_attributes_truncated_total 0\n"));
    }

    #[test]
    fn renders_incremented_values_under_the_right_labels() {
        let m = Metrics::default();
        Metrics::inc(&m.requests_accepted, 3);
        Metrics::inc(&m.requests_accepted, 2);
        Metrics::inc(&m.spans_rejected, 7);
        Metrics::inc(&m.events_rejected, 1);
        Metrics::inc(&m.queued_bytes, 42);
        let out = m.render();
        assert!(out.contains("controlhub_ingest_requests_total{result=\"accepted\"} 5\n"));
        assert!(out.contains("controlhub_ingest_items_total{signal=\"spans\",result=\"rejected\"} 7\n"));
        assert!(out.contains("controlhub_ingest_items_total{signal=\"events\",result=\"rejected\"} 1\n"));
        assert!(out.contains("controlhub_ingest_items_total{signal=\"spans\",result=\"accepted\"} 0\n"));
        assert!(out.contains("controlhub_ingest_queued_bytes 42\n"));
    }

    #[test]
    fn every_series_is_unique() {
        let out = Metrics::default().render();
        let series: std::collections::BTreeSet<_> =
            out.lines().filter_map(|l| l.rsplit_once(' ')).map(|(s, _)| s).collect();
        assert_eq!(series.len(), out.lines().count());
    }
}
