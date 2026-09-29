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

#[cfg(test)]
mod tests {
    use super::*;

    fn span(id: &str, parent: Option<&str>, kind: u8, start: i64, end: i64, err: bool) -> SpanItem {
        SpanItem {
            trace_id: "t".into(),
            span_id: id.into(),
            parent_span_id: parent.map(Into::into),
            service_id: "svc".into(),
            service_name: "svc".into(),
            target_entity_id: None,
            name: "op".into(),
            kind,
            start_ns: start,
            end_ns: end,
            is_error: err,
        }
    }

    #[test]
    fn window_is_open_at_the_start_and_closed_at_the_end() {
        let spans = [span("a", None, 1, 10, 20, true)];
        let at = |t, w| metrics_at(&spans, &[], t, w).remove("svc").unwrap_or_default();
        assert_eq!(at(20, 10), RefNodeMetrics { started: 0, completed: 1, errors: 1, ..Default::default() });
        assert_eq!(at(19, 10), RefNodeMetrics { active: 1, started: 1, ..Default::default() });
        assert_eq!(at(10, 0), RefNodeMetrics { active: 1, ..Default::default() });
        assert_eq!(at(9, 100), RefNodeMetrics::default());
        assert_eq!(metrics_at(&spans, &[], i64::MIN, i64::MAX)["svc"], RefNodeMetrics::default());
    }

    #[test]
    fn only_entry_spans_count_and_duplicates_collapse() {
        let spans = [
            span("root", None, 3, 0, 5, false),
            span("server", Some("x"), 2, 0, 5, false),
            span("consumer", Some("x"), 5, 0, 5, false),
            span("client", Some("x"), 3, 0, 5, true),
            span("server", Some("x"), 2, 0, 5, false),
        ];
        let logs = [
            LogItem {
                id: "l".into(),
                service_id: "other".into(),
                service_name: "o".into(),
                time_ns: 3,
                severity: 17,
                trace_id: None,
            },
            LogItem {
                id: "l".into(),
                service_id: "other".into(),
                service_name: "o".into(),
                time_ns: 3,
                severity: 17,
                trace_id: None,
            },
        ];
        let m = metrics_at(&spans, &logs, 5, 10);
        assert_eq!(m["svc"], RefNodeMetrics { started: 3, completed: 3, ..Default::default() });
        assert_eq!(m["other"], RefNodeMetrics { logs: 1, error_logs: 1, ..Default::default() });
    }
}
