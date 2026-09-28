#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_arguments)]

use super::*;
use crate::reference;
use proptest::prelude::*;

const SEC: i64 = 1_000_000_000;
const BASE: i64 = 1_790_000_000 * SEC;

fn span(trace: &str, id: &str, parent: Option<&str>, svc: &str, kind: u8, start: i64, end: i64, err: bool) -> SpanItem {
    SpanItem {
        trace_id: trace.into(),
        span_id: id.into(),
        parent_span_id: parent.map(Into::into),
        service_id: format!("svc_{svc}"),
        service_name: svc.into(),
        target_entity_id: None,
        name: format!("{svc} op"),
        kind,
        start_ns: BASE + start,
        end_ns: BASE + end,
        is_error: err,
    }
}

fn log(id: &str, svc: &str, t: i64, sev: u8) -> LogItem {
    LogItem {
        id: id.into(),
        service_id: format!("svc_{svc}"),
        service_name: svc.into(),
        time_ns: BASE + t,
        severity: sev,
        trace_id: None,
    }
}

fn demo_items() -> Vec<TimelineItem> {
    vec![
        TimelineItem::Span(span("t1", "a1", None, "gateway", 2, 0, 10 * SEC, false)),
        TimelineItem::Span(span("t1", "a2", Some("a1"), "gateway", 3, SEC, 9 * SEC, false)),
        TimelineItem::Span(span("t1", "b1", Some("a2"), "orders", 2, 2 * SEC, 8 * SEC, true)),
        TimelineItem::Log(log("l1", "orders", 3 * SEC, 17)),
        TimelineItem::Event(EventItem {
            id: "e1".into(),
            event_type: "request.started".into(),
            service_id: Some("svc_gateway".into()),
            service_name: Some("gateway".into()),
            time_ns: BASE,
            trace_id: Some("t1".into()),
        }),
    ]
}

fn engine_with(items: Vec<TimelineItem>) -> Engine {
    let mut e = Engine::new(EngineConfig::default());
    e.apply_batch(Batch { items }).unwrap();
    e
}

#[test]
fn reconstructs_state_and_observed_edge() {
    let mut e = engine_with(demo_items());
    assert_eq!(e.seek(BASE + 5 * SEC, 1), SeekOutcome::Applied);
    let topo = e.topology();
    assert_eq!(topo.nodes.len(), 2);
    assert_eq!(topo.edges.len(), 1, "gateway -> orders observed from parent/child spans");
    assert_eq!(topo.edges[0].provenance, ProvenanceDto::Observed);

    let orders = e.node_metrics("svc_orders", None).unwrap();
    assert_eq!(orders.active, 1);
    assert_eq!(orders.window_error_logs, 1);

    let vs = e.visible_state();
    assert_eq!(vs.summary.total_active, 2);
    assert_eq!(vs.node_data.len(), 2 * NODE_STRIDE);
    assert_eq!(vs.edge_data.len(), EDGE_STRIDE);
    // edge active count at t=5s
    assert_eq!(vs.edge_data[4], 1.0);

    e.seek(BASE + 9 * SEC, 2);
    let orders = e.node_metrics("svc_orders", None).unwrap();
    assert_eq!(orders.active, 0);
    assert_eq!(orders.window_errors, 1);
    assert_eq!(orders.status, HealthStatusDto::Failing);
}

#[test]
fn unknown_is_default_without_data() {
    let mut e = engine_with(demo_items());
    e.seek(BASE - 100 * SEC, 1);
    let m = e.node_metrics("svc_gateway", None).unwrap();
    assert_eq!(m.status, HealthStatusDto::Unknown);
}

#[test]
fn stale_generation_is_rejected() {
    let mut e = engine_with(demo_items());
    assert_eq!(e.seek(BASE + 5 * SEC, 10), SeekOutcome::Applied);
    assert_eq!(e.seek(BASE, 9), SeekOutcome::Stale);
    assert_eq!(e.current_time(), Some(BASE + 5 * SEC));
    assert_eq!(e.diagnostics().stale_seeks_ignored, 1);
}

#[test]
fn step_navigation_visits_recorded_times() {
    let mut e = engine_with(demo_items());
    assert_eq!(e.next_event_time(BASE - 1), Some(BASE));
    assert_eq!(e.next_event_time(BASE), Some(BASE + SEC));
    assert_eq!(e.prev_event_time(BASE + SEC), Some(BASE));
    assert_eq!(e.next_event_time(BASE + 10 * SEC), None);
}

#[test]
fn duplicates_do_not_double_count() {
    let mut items = demo_items();
    items.extend(demo_items());
    let mut e = engine_with(items);
    e.seek(BASE + 9 * SEC, 1);
    let orders = e.node_metrics("svc_orders", None).unwrap();
    assert_eq!(orders.window_errors, 1);
    assert_eq!(orders.window_error_logs, 1);
    assert!(e.diagnostics().duplicates_ignored >= 5);
}

#[test]
fn late_parent_resolves_edge() {
    let mut e = Engine::new(EngineConfig::default());
    e.apply_batch(Batch {
        items: vec![TimelineItem::Span(span("t1", "b1", Some("a2"), "orders", 2, 2 * SEC, 8 * SEC, false))],
    })
    .unwrap();
    e.seek(BASE + 5 * SEC, 1);
    assert_eq!(e.diagnostics().unresolved_parents, 1);
    assert_eq!(e.topology().edges.len(), 0);
    e.apply_batch(Batch { items: vec![TimelineItem::Span(span("t1", "a2", None, "gateway", 3, SEC, 9 * SEC, false))] })
        .unwrap();
    e.seek(BASE + 5 * SEC, 2);
    assert_eq!(e.diagnostics().unresolved_parents, 0);
    assert_eq!(e.topology().edges.len(), 1);
}

#[test]
fn bounded_window_evicts_oldest() {
    let mut e = Engine::new(EngineConfig { max_items: 10, ..EngineConfig::default() });
    let items = (0..50).map(|i| TimelineItem::Log(log(&format!("l{i}"), "svc", i * SEC, 9))).collect();
    e.apply_batch(Batch { items }).unwrap();
    let d = e.diagnostics();
    assert!(d.logs <= 10);
    assert_eq!(d.evicted_items, 40);
    assert!(d.evicted_before_ns.is_some());
}

#[test]
fn viewport_culls_offscreen_nodes() {
    let mut e = engine_with(demo_items());
    e.seek(BASE + 5 * SEC, 1);
    let all = e.visible_state().summary.visible_nodes;
    e.set_viewport(Viewport { x: 100_000.0, y: 100_000.0, width: 100.0, height: 100.0, scale: 1.0 });
    assert_eq!(all, 2);
    assert_eq!(e.visible_state().summary.visible_nodes, 0);
}

#[test]
fn trace_selection_highlights_path() {
    let mut e = engine_with(demo_items());
    e.seek(BASE + 5 * SEC, 1);
    e.set_filters(Filters { selected_trace_id: Some("t1".into()), ..Filters::default() });
    let vs = e.visible_state();
    let flags = vs.edge_data[7] as u32;
    assert!(flags & FLAG_HIGHLIGHT != 0);
}

#[test]
fn compare_reports_changes() {
    let mut e = engine_with(demo_items());
    let diffs = e.compare(BASE - SEC, BASE + 9 * SEC);
    assert!(diffs.iter().any(|d| d.name == "orders" && d.after.window_errors == 1));
}

#[test]
fn layout_is_stable_when_new_nodes_appear() {
    let mut e = engine_with(demo_items());
    let before: Vec<(String, f32, f32)> = e.topology().nodes.iter().map(|n| (n.id.clone(), n.x, n.y)).collect();
    e.apply_batch(Batch { items: vec![TimelineItem::Log(log("lx", "payments", 4 * SEC, 9))] }).unwrap();
    let after = e.topology();
    for (id, x, y) in before {
        let n = after.nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!((n.x, n.y), (x, y), "existing node {id} moved");
    }
}

#[test]
fn json_contract_parses_decimal_string_timestamps() {
    let json = r#"{"items":[{"type":"log","id":"x","service_id":"svc_a","service_name":"a","time_ns":"1790000000123456789","severity":9}]}"#;
    let mut e = Engine::new(EngineConfig::default());
    e.apply_batch_json(json).unwrap();
    assert_eq!(e.diagnostics().first_item_ns, Some(1_790_000_000_123_456_789));
}

// ------------------------------------------------------------- properties

fn arb_items() -> impl Strategy<Value = (Vec<SpanItem>, Vec<LogItem>)> {
    let svc = prop::sample::select(vec!["a", "b", "c"]);
    let spans = prop::collection::vec(
        (0u32..40, svc.clone(), 0i64..100, 0i64..30, any::<bool>(), prop::sample::select(vec![1u8, 2, 3])),
        0..60,
    )
    .prop_map(|v| {
        v.into_iter()
            .map(|(id, s, start, dur, err, kind)| {
                let parent = if id % 3 == 0 { None } else { Some(format!("p{}", id / 3)) };
                span(
                    &format!("t{}", id % 7),
                    &format!("s{id}"),
                    parent.as_deref(),
                    s,
                    kind,
                    start * SEC,
                    (start + dur) * SEC,
                    err,
                )
            })
            .collect::<Vec<_>>()
    });
    let logs = prop::collection::vec((0u32..40, svc, 0i64..130, prop::sample::select(vec![9u8, 13, 17])), 0..60)
        .prop_map(|v| v.into_iter().map(|(id, s, t, sev)| log(&format!("l{id}"), s, t * SEC, sev)).collect::<Vec<_>>());
    (spans, logs)
}

fn dedup_last_wins(spans: &[SpanItem], logs: &[LogItem]) -> (Vec<SpanItem>, Vec<LogItem>) {
    // The engine keeps the last write per ID; mirror that for the reference.
    let mut s = std::collections::BTreeMap::new();
    for x in spans {
        s.insert((x.trace_id.clone(), x.span_id.clone()), x.clone());
    }
    let mut l = std::collections::BTreeMap::new();
    for x in logs {
        l.insert(x.id.clone(), x.clone());
    }
    (s.into_values().collect(), l.into_values().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Engine (checkpoints + replay) matches the naive reference at any time.
    #[test]
    fn engine_matches_reference((spans, logs) in arb_items(), t in 0i64..140, small_budget in any::<bool>()) {
        let (spans, logs) = dedup_last_wins(&spans, &logs);
        let cfg = EngineConfig {
            rolling_window_ns: 20 * SEC,
            checkpoint_budget_bytes: if small_budget { 1 } else { 16 << 20 },
            ..EngineConfig::default()
        };
        let mut e = Engine::new(cfg);
        let items = spans.iter().cloned().map(TimelineItem::Span)
            .chain(logs.iter().cloned().map(TimelineItem::Log)).collect();
        e.apply_batch(Batch { items }).unwrap();
        let tt = BASE + t * SEC;
        e.seek(tt, 1);
        let expected = reference::metrics_at(&spans, &logs, tt, 20 * SEC);
        for (svc, m) in expected {
            let got = e.node_metrics(&svc, None).unwrap();
            prop_assert_eq!(i64::from(got.active), m.active, "active {}", svc);
            prop_assert_eq!(i64::from(got.window_requests), m.started, "started {}", svc);
            prop_assert_eq!(i64::from(got.window_completed), m.completed, "completed {}", svc);
            prop_assert_eq!(i64::from(got.window_errors), m.errors, "errors {}", svc);
            prop_assert_eq!(i64::from(got.window_logs), m.logs, "logs {}", svc);
            prop_assert_eq!(i64::from(got.window_error_logs), m.error_logs, "error logs {}", svc);
        }
    }

    /// Arrival order, batch splitting and duplicate delivery do not change the result.
    #[test]
    fn order_and_duplicates_invariant((spans, logs) in arb_items(), split in 0usize..120, t in 0i64..140, seed in any::<u64>()) {
        let (spans, logs) = dedup_last_wins(&spans, &logs);
        let items: Vec<TimelineItem> = spans.iter().cloned().map(TimelineItem::Span)
            .chain(logs.iter().cloned().map(TimelineItem::Log)).collect();

        let mut a = Engine::new(EngineConfig::default());
        a.apply_batch(Batch { items: items.clone() }).unwrap();

        // Shuffle deterministically, duplicate everything, split into two batches.
        let mut shuffled = items.clone();
        shuffled.extend(items.iter().cloned());
        let mut x = seed | 1;
        for i in (1..shuffled.len()).rev() {
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            shuffled.swap(i, (x % (i as u64 + 1)) as usize);
        }
        let cut = split.min(shuffled.len());
        let second = shuffled.split_off(cut);
        let mut b = Engine::new(EngineConfig::default());
        b.apply_batch(Batch { items: shuffled }).unwrap();
        b.apply_batch(Batch { items: second }).unwrap();

        let tt = BASE + t * SEC;
        a.seek(tt, 1);
        b.seek(tt, 1);
        for n in a.topology().nodes {
            prop_assert_eq!(a.node_metrics(&n.id, None), b.node_metrics(&n.id, None));
        }
        let edges = |e: &Engine| {
            let topo = e.topology();
            let mut v: Vec<(String, String)> = topo.edges.iter()
                .map(|x| (topo.nodes[x.source as usize].id.clone(), topo.nodes[x.target as usize].id.clone()))
                .collect();
            v.sort();
            v
        };
        prop_assert_eq!(edges(&a), edges(&b));
    }
}
