//! Naive reference implementation used to validate the engine and as a
//! baseline in benchmarks. Scans every item on every query — O(n) per seek.

use crate::types::{LogItem, SpanItem};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefNodeMetrics {
    pub active: i64,
    pub started: i64,
    pub completed: i64,
    pub errors: i64,
    pub logs: i64,
    pub error_logs: i64,
}

fn is_entry(s: &SpanItem) -> bool {
    s.kind == 2 || s.kind == 5 || s.parent_span_id.is_none()
}

/// Per-service metrics at `t` over the window `(t - window, t]`.
/// Input may contain duplicates (same trace/span or log id): they are collapsed first.
pub fn metrics_at(spans: &[SpanItem], logs: &[LogItem], t: i64, window: i64) -> BTreeMap<String, RefNodeMetrics> {
    let mut uniq_spans: BTreeMap<(&str, &str), &SpanItem> = BTreeMap::new();
    for s in spans {
        uniq_spans.insert((&s.trace_id, &s.span_id), s);
    }
    let mut uniq_logs: BTreeMap<&str, &LogItem> = BTreeMap::new();
    for l in logs {
        uniq_logs.insert(&l.id, l);
    }
    let from = t.saturating_sub(window);
    let in_window = |x: i64| x > from && x <= t;
    let mut out: BTreeMap<String, RefNodeMetrics> = BTreeMap::new();
    for s in uniq_spans.values() {
        let m = out.entry(s.service_id.clone()).or_default();
        if !is_entry(s) {
            continue;
        }
        if s.start_ns <= t && s.end_ns > t {
            m.active += 1;
        }
        if in_window(s.start_ns) {
            m.started += 1;
        }
        if in_window(s.end_ns) {
            m.completed += 1;
            if s.is_error {
                m.errors += 1;
            }
        }
    }
    for l in uniq_logs.values() {
        let m = out.entry(l.service_id.clone()).or_default();
        if in_window(l.time_ns) {
            m.logs += 1;
            if l.severity >= 17 {
                m.error_logs += 1;
            }
        }
    }
    out
}
