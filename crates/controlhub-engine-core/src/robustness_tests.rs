//! Engine-level tests for invalid input, filters, frame buffer invariants, eviction,
//! checkpoints across seeks and live merges, checked against naive oracles.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_arguments)]

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::*;
use crate::reference;
use proptest::prelude::*;

const SEC: i64 = 1_000_000_000;
const MS: i64 = 1_000_000;
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

fn raw_span(id: &str, svc: &str, start: i64, end: i64) -> SpanItem {
    SpanItem { start_ns: start, end_ns: end, ..span("t", id, None, svc, 2, 0, 0, false) }
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

fn event(id: &str, svc: Option<&str>, ty: &str, t: i64) -> EventItem {
    EventItem {
        id: id.into(),
        event_type: ty.into(),
        service_id: svc.map(|s| format!("svc_{s}")),
        service_name: svc.map(Into::into),
        time_ns: BASE + t,
        trace_id: None,
    }
}

fn db_node() -> TopologySnapshot {
    TopologySnapshot {
        nodes: vec![TopologyNode {
            id: "res_db".into(),
            name: "postgres".into(),
            kind: EntityKindDto::Database,
            provenance: ProvenanceDto::Inferred,
        }],
        edges: vec![],
    }
}

/// gateway (entry) -> orders (server, failing) -> postgres (client span to a resource),
/// plus a healthy `billing` service with its own traffic and no edges.
fn system() -> Engine {
    let mut e = Engine::new(EngineConfig::default());
    e.load_topology(db_node());
    let mut items = vec![
        TimelineItem::Span(span("t1", "g1", None, "gateway", 2, 0, 10 * MS, false)),
        TimelineItem::Span(span("t1", "g2", Some("g1"), "gateway", 3, MS, 9 * MS, false)),
        TimelineItem::Span(span("t1", "o1", Some("g2"), "orders", 2, 2 * MS, 8 * MS, true)),
        TimelineItem::Span(SpanItem {
            target_entity_id: Some("res_db".into()),
            ..span("t1", "d1", Some("o1"), "orders", 3, 3 * MS, 6 * MS, true)
        }),
    ];
    for i in 0..5 {
        items.push(TimelineItem::Span(span(&format!("b{i}"), "r", None, "billing", 2, i * MS, (i + 1) * MS, false)));
    }
    e.apply_batch(Batch { items }).unwrap();
    e
}

fn node_idx(e: &Engine, id: &str) -> u32 {
    e.topology().nodes.iter().find(|n| n.id == id).unwrap().index
}

fn visible_ids(e: &Engine, vs: &VisibleState) -> BTreeSet<String> {
    let topo = e.topology();
    vs.node_indices.iter().map(|&i| topo.nodes[i as usize].id.clone()).collect()
}

fn visible_edge_pairs(e: &Engine, vs: &VisibleState) -> BTreeSet<(String, String)> {
    let topo = e.topology();
    vs.edge_indices
        .iter()
        .map(|&i| {
            let x = &topo.edges[i as usize];
            (topo.nodes[x.source as usize].id.clone(), topo.nodes[x.target as usize].id.clone())
        })
        .collect()
}

/// Structural invariants every frame must satisfy, whatever the input and filters.
fn check_frame(e: &Engine, vs: &VisibleState) -> Result<(), TestCaseError> {
    let topo = e.topology();
    let (n, m) = (topo.nodes.len(), topo.edges.len());
    prop_assert_eq!(vs.node_data.len(), vs.node_indices.len() * NODE_STRIDE);
    prop_assert_eq!(vs.edge_data.len(), vs.edge_indices.len() * EDGE_STRIDE);
    prop_assert_eq!(vs.hop_data.len() % HOP_STRIDE, 0);
    prop_assert!(vs.node_indices.windows(2).all(|w| w[0] < w[1]), "node indices sorted and unique");
    prop_assert!(vs.edge_indices.windows(2).all(|w| w[0] < w[1]), "edge indices sorted and unique");
    prop_assert!(vs.node_indices.iter().all(|&i| (i as usize) < n));
    prop_assert!(vs.edge_indices.iter().all(|&i| (i as usize) < m));
    prop_assert!(vs.changed_nodes.iter().all(|&i| (i as usize) < n));
    prop_assert_eq!(vs.summary.visible_nodes as usize, vs.node_indices.len());
    prop_assert_eq!(vs.summary.visible_edges as usize, vs.edge_indices.len());
    prop_assert_eq!(vs.summary.hops_shown as usize * HOP_STRIDE, vs.hop_data.len());
    prop_assert!(vs.summary.hops_shown <= vs.summary.hops_in_window);
    prop_assert!(vs.summary.hops_shown <= e.config().max_hops);
    prop_assert!(vs.node_data.iter().chain(&vs.edge_data).chain(&vs.hop_data).all(|f| f.is_finite()));

    let visible: HashSet<u32> = vs.node_indices.iter().copied().collect();
    for (k, &i) in vs.edge_indices.iter().enumerate() {
        let x = &topo.edges[i as usize];
        prop_assert!(visible.contains(&x.source) || visible.contains(&x.target), "edge {} has no visible end", i);
        // Calls count at span start and errors at span end, so the ratio may exceed 1 in a window.
        let ratio = vs.edge_data[k * EDGE_STRIDE + 6];
        prop_assert!(ratio >= 0.0, "error ratio {}", ratio);
    }
    for c in vs.node_data.chunks(NODE_STRIDE) {
        prop_assert!(c[2] > 0.0, "radius");
        prop_assert!(c[3] <= 3.0 && c[3] >= 0.0, "status");
        prop_assert!(c[4] >= 0.0 && c[5] >= 0.0, "counters");
        prop_assert!((c[6] as u32) < 64, "flags");
    }
    let shown: HashSet<u32> = vs.edge_indices.iter().copied().collect();
    for h in vs.hop_data.chunks(HOP_STRIDE) {
        prop_assert!(shown.contains(&(h[0] as u32)), "hop on a hidden edge: {:?}", h);
        prop_assert!(h[1] <= h[2] && h[2] <= h[3] && h[3] <= h[4], "hop times out of order: {:?}", h);
        prop_assert!((h[5] as u32) < 32, "hop flags");
    }
    Ok(())
}

fn frame(e: &mut Engine) -> Result<(), TestCaseError> {
    let vs = e.visible_state();
    check_frame(e, &vs)
}

// ------------------------------------------------------------------ invalid input

#[test]
fn invalid_span_rejects_the_whole_batch() {
    let mut e = system();
    e.seek(BASE + 5 * MS, 1);
    let before = e.visible_state();
    let (nodes, items) = (e.topology().nodes.len(), e.item_count());
    let bad = Batch {
        items: vec![
            TimelineItem::Log(log("new", "newsvc", 4 * MS, 17)),
            TimelineItem::Span(span("t1", "zz", None, "other", 2, 5 * MS, 4 * MS, false)),
        ],
    };
    assert!(matches!(e.apply_batch(bad), Err(EngineError::InvalidInput(_))));
    assert_eq!(e.topology().nodes.len(), nodes, "no node from a rejected batch");
    assert_eq!(e.item_count(), items, "no item from a rejected batch");
    e.seek(BASE + 5 * MS, 2);
    let after = e.visible_state();
    assert_eq!(after.node_data, before.node_data);
    assert_eq!(after.edge_data, before.edge_data);

    // The engine keeps working and still agrees with a fresh build.
    let more = vec![
        TimelineItem::Log(log("new", "newsvc", 4 * MS, 17)),
        TimelineItem::Span(span("t9", "x", None, "orders", 2, 4 * MS, 7 * MS, false)),
    ];
    e.apply_batch(Batch { items: more.clone() }).unwrap();
    let mut fresh = system();
    fresh.apply_batch(Batch { items: more }).unwrap();
    for id in ["svc_gateway", "svc_orders", "svc_newsvc", "res_db", "svc_billing"] {
        assert_eq!(e.node_metrics(id, Some(BASE + 7 * MS)), fresh.node_metrics(id, Some(BASE + 7 * MS)), "{id}");
    }
}

#[test]
fn zero_length_spans_are_valid() {
    let mut e = Engine::new(EngineConfig::default());
    e.apply_batch(Batch { items: vec![TimelineItem::Span(span("t", "a", None, "api", 2, SEC, SEC, true))] }).unwrap();
    let m = e.node_metrics("svc_api", Some(BASE + SEC)).unwrap();
    assert_eq!((m.active, m.window_requests, m.window_completed, m.window_errors), (0, 1, 1, 1));
    assert_eq!(m.avg_latency_ms, Some(0.0));
}

#[test]
fn extreme_timestamps_never_panic() {
    let mut e = Engine::new(EngineConfig { rolling_window_ns: i64::MAX, ..EngineConfig::default() });
    e.load_topology(db_node());
    let items = vec![
        TimelineItem::Span(raw_span("full", "a", i64::MIN, i64::MAX)),
        TimelineItem::Span(raw_span("full2", "a", i64::MIN, i64::MAX)),
        TimelineItem::Span(SpanItem {
            parent_span_id: Some("full".into()),
            kind: 2,
            ..raw_span("child", "b", i64::MAX, i64::MAX)
        }),
        TimelineItem::Span(SpanItem {
            parent_span_id: Some("full".into()),
            kind: 3,
            target_entity_id: Some("res_db".into()),
            ..raw_span("leaf", "a", i64::MIN, i64::MAX)
        }),
        TimelineItem::Span(raw_span("neg", "c", -5 * SEC, -SEC)),
        TimelineItem::Log(LogItem { time_ns: i64::MIN, ..log("lmin", "a", 0, 17) }),
        TimelineItem::Log(LogItem { time_ns: i64::MAX, ..log("lmax", "a", 0, 9) }),
        TimelineItem::Event(EventItem { time_ns: i64::MIN, ..event("emin", Some("a"), "deploy", 0) }),
        TimelineItem::Event(EventItem { time_ns: i64::MAX, ..event("emax", None, "request.x", 0) }),
    ];
    e.apply_batch(Batch { items }).unwrap();
    e.set_hop_window(i64::MAX, i64::MIN);
    e.set_filters(Filters {
        selected_trace_id: Some("t".into()),
        muted_services: vec!["svc_b".into()],
        ..Filters::default()
    });
    for (g, t) in [i64::MIN, -1, 0, 1, i64::MAX / 2, i64::MAX].into_iter().enumerate() {
        assert_eq!(e.seek(t, g as u64 + 1), SeekOutcome::Applied);
        let vs = e.visible_state();
        check_frame(&e, &vs).unwrap();
        e.node_metrics("svc_a", None).unwrap();
        e.prev_event_time(t);
        e.next_event_time(t);
    }
    e.compare(i64::MIN, i64::MAX);
    for (from, to) in [(i64::MIN, i64::MAX), (i64::MAX - 1, i64::MAX), (i64::MIN, i64::MIN + 1), (0, i64::MAX)] {
        let t = e.timeline_tracks(from, to, 7, 100);
        assert_eq!(t.requests.len(), 7);
        assert!(t.latency_p95_ms.iter().flatten().all(|v| v.is_finite() && *v >= 0.0));
    }
    let d = e.diagnostics();
    assert_eq!((d.first_item_ns, d.last_item_ns), (Some(i64::MIN), Some(i64::MAX)));
    e.evict_before(i64::MAX);
    assert_eq!(e.item_count(), 6, "items ending at i64::MAX survive");
    e.evict_before(i64::MIN);
    e.seek(i64::MAX, 100);
    frame(&mut e).unwrap();
}

#[test]
fn empty_and_unicode_strings_are_plain_data() {
    let mut e = Engine::new(EngineConfig::default());
    let items = vec![
        TimelineItem::Span(SpanItem {
            service_id: String::new(),
            service_name: String::new(),
            ..span("", "", None, "x", 2, 0, SEC, false)
        }),
        TimelineItem::Span(SpanItem {
            service_id: "svc_ÄPI".into(),
            service_name: "Сервис 🚀 <script>alert(1)</script>".into(),
            ..span("t", "u", None, "x", 2, 0, SEC, true)
        }),
        TimelineItem::Log(LogItem { id: String::new(), ..log("", "x", 0, 17) }),
    ];
    e.apply_batch(Batch { items }).unwrap();
    e.seek(BASE + SEC, 1);
    let topo = e.topology();
    assert!(topo.nodes.iter().any(|n| n.name == "Сервис 🚀 <script>alert(1)</script>"), "names are kept verbatim");
    assert!(e.node_metrics("", None).is_some());
    // Case-insensitive search works on non-ASCII names.
    e.set_filters(Filters { search: "сервис".into(), ..Filters::default() });
    let vs = e.visible_state();
    assert_eq!(visible_ids(&e, &vs), BTreeSet::from(["svc_ÄPI".to_owned()]));
    e.set_filters(Filters { search: "🚀 <SCRIPT>".into(), ..Filters::default() });
    assert_eq!(e.visible_state().summary.visible_nodes, 1);
}

#[test]
fn malformed_json_is_an_error_and_changes_nothing() {
    let mut e = system();
    e.seek(BASE + 5 * MS, 1);
    let (items, nodes) = (e.item_count(), e.topology().nodes.len());
    let bad_batches = [
        "",
        "{",
        "null",
        "[]",
        r#"{"items":null}"#,
        r#"{"items":[{"type":"span"}]}"#,
        r#"{"items":[{"type":"metric","id":"x"}]}"#,
        r#"{"items":[{"id":"x","service_id":"a","service_name":"a","time_ns":"1","severity":9}]}"#,
        r#"{"items":[{"type":"log","id":"x","service_id":"a","service_name":"a","time_ns":"abc","severity":9}]}"#,
        r#"{"items":[{"type":"log","id":"x","service_id":"a","service_name":"a","time_ns":"99999999999999999999","severity":9}]}"#,
        r#"{"items":[{"type":"log","id":"x","service_id":"a","service_name":"a","time_ns":"1","severity":300}]}"#,
        r#"{"items":[{"type":"log","id":"x","service_id":"a","service_name":"a","time_ns":1.5,"severity":9}]}"#,
        r#"{"items":[{"type":"span","trace_id":"t","span_id":"s","service_id":"a","service_name":"a","name":"n","kind":-1,"start_ns":"1","end_ns":"2","is_error":false}]}"#,
        r#"{"items":[{"type":"span","trace_id":"t","span_id":"s","service_id":"a","service_name":"a","name":"n","kind":2,"start_ns":"5","end_ns":"2","is_error":false}]}"#,
        r#"{"items":[{"type":"log","id":"x","service_id":"a","service_name":"a","time_ns":"1","severity":9}]} trailing"#,
    ];
    for json in bad_batches {
        let err = e.apply_batch_json(json);
        assert!(matches!(err, Err(EngineError::InvalidInput(_))), "accepted: {json}");
    }
    let bad_topologies = [
        "",
        "{}",
        r#"{"nodes":[],"edges":[{"source":"a"}]}"#,
        r#"{"nodes":[{"id":"a","name":"a","kind":"spaceship","provenance":"observed"}],"edges":[]}"#,
    ];
    for json in bad_topologies {
        assert!(e.load_topology_json(json).is_err(), "accepted topology: {json}");
    }
    assert_eq!((e.item_count(), e.topology().nodes.len()), (items, nodes));
    // Integer timestamps and unknown fields are accepted.
    e.apply_batch_json(
        r#"{"items":[{"type":"log","id":"x","service_id":"svc_orders","service_name":"orders","time_ns":5,"severity":9,"extra":1}]}"#,
    )
    .unwrap();
    // Edges to unknown nodes are skipped, not an error.
    e.load_topology_json(
        r#"{"nodes":[],"edges":[{"source":"nope","target":"svc_orders","relation":"calls","provenance":"declared"}]}"#,
    )
    .unwrap();
}

fn json_fragment() -> impl Strategy<Value = String> {
    let pieces = prop::sample::select(vec![
        "{",
        "}",
        "[",
        "]",
        ",",
        ":",
        "\"items\"",
        "\"type\"",
        "\"span\"",
        "\"log\"",
        "\"event\"",
        "\"id\"",
        "\"time_ns\"",
        "\"start_ns\"",
        "\"end_ns\"",
        "\"1\"",
        "\"-9223372036854775808\"",
        "\"9223372036854775807\"",
        "1",
        "-1",
        "1e999",
        "null",
        "true",
        "\"nodes\"",
        "\"edges\"",
        "\"\\u0000\"",
        "\"\\ud800\"",
        "\"😀\"",
    ]);
    prop::collection::vec(pieces, 0..40).prop_map(|v| v.concat())
}

fn valid_batch_json() -> String {
    let items = vec![
        TimelineItem::Span(span("t1", "a", None, "gateway", 2, 0, SEC, true)),
        TimelineItem::Span(span("t1", "b", Some("a"), "orders", 2, 0, SEC, false)),
        TimelineItem::Log(log("l", "orders", 0, 17)),
        TimelineItem::Event(event("e", Some("orders"), "deploy", 0)),
    ];
    serde_json::to_string(&Batch { items }).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Arbitrary text never panics the JSON entry points.
    #[test]
    fn arbitrary_text_never_panics(s in any::<String>(), frag in json_fragment()) {
        let mut e = system();
        for input in [&s, &frag] {
            let _ = e.apply_batch_json(input);
            let _ = e.load_topology_json(input);
        }
        e.seek(BASE, 1);
        frame(&mut e)?;
    }

    /// Truncated or byte-flipped valid batches either parse or fail cleanly.
    #[test]
    fn mutated_valid_json_never_panics(cut in 0usize..400, flip in 0usize..400, byte in any::<u8>()) {
        let json = valid_batch_json();
        let mut bytes = json.into_bytes();
        if let Some(b) = bytes.get_mut(flip) {
            *b = byte;
        }
        bytes.truncate(cut.max(1));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut e = Engine::new(EngineConfig::default());
        let before = e.item_count();
        if e.apply_batch_json(&text).is_err() {
            prop_assert_eq!(e.item_count(), before);
        }
        e.seek(BASE, 1);
        frame(&mut e)?;
    }

    /// Arbitrary timestamps (including the i64 extremes) never overflow anywhere.
    #[test]
    fn arbitrary_timestamps_never_panic(
        raw in prop::collection::vec((any::<i64>(), any::<i64>(), any::<bool>(), 0u8..6), 1..12),
        seeks in prop::collection::vec(any::<i64>(), 1..4),
        window in any::<i64>(),
        hop in (any::<i64>(), any::<i64>()),
    ) {
        let mut e = Engine::new(EngineConfig { rolling_window_ns: window, ..EngineConfig::default() });
        e.load_topology(db_node());
        let mut items = Vec::new();
        for (i, (a, b, err, kind)) in raw.iter().copied().enumerate() {
            let (start, end) = (a.min(b), a.max(b));
            let parent = (i > 0).then(|| format!("s{}", i - 1));
            items.push(TimelineItem::Span(SpanItem {
                trace_id: "t".into(),
                span_id: format!("s{i}"),
                parent_span_id: parent,
                service_id: format!("svc_{}", i % 3),
                service_name: format!("{}", i % 3),
                target_entity_id: (kind == 3 && i % 2 == 0).then(|| "res_db".into()),
                name: "op".into(),
                kind,
                start_ns: start,
                end_ns: end,
                is_error: err,
            }));
            items.push(TimelineItem::Log(LogItem { time_ns: a, ..log(&format!("l{i}"), "0", 0, 17) }));
        }
        e.apply_batch(Batch { items }).unwrap();
        e.set_hop_window(hop.0, hop.1);
        e.set_filters(Filters { selected_trace_id: Some("t".into()), muted_services: vec!["svc_1".into()], ..Filters::default() });
        for (g, t) in seeks.iter().copied().enumerate() {
            e.seek(t, g as u64 + 1);
            let vs = e.visible_state();
            check_frame(&e, &vs)?;
            let tracks = e.timeline_tracks(t.min(window), t.max(window), 5, 50);
            prop_assert!(tracks.clips.iter().all(|c| c.start_ms.is_finite() && c.end_ms.is_finite()));
        }
        e.compare(seeks[0], window);
        e.diagnostics();
        e.evict_before(seeks[0]);
        e.seek(window, 99);
        frame(&mut e)?;
    }
}

// ------------------------------------------------------------------ filters

#[test]
fn search_is_a_plain_case_insensitive_substring() {
    let mut e = system();
    e.seek(BASE + 5 * MS, 1);
    let ids = |e: &mut Engine, search: &str| {
        e.set_filters(Filters { search: search.into(), ..Filters::default() });
        let vs = e.visible_state();
        visible_ids(e, &vs)
    };
    assert_eq!(ids(&mut e, "ORD"), BTreeSet::from(["svc_orders".to_owned()]));
    assert_eq!(ids(&mut e, "gres"), BTreeSet::from(["res_db".to_owned()]), "matches the display name");
    assert_eq!(ids(&mut e, "res_d"), BTreeSet::from(["res_db".to_owned()]), "matches the id");
    assert_eq!(ids(&mut e, "svc_").len(), 3);
    for pattern in [".*", "^svc", "(", "[a-z]+", "\\", "o|g", "%", "*"] {
        assert!(ids(&mut e, pattern).is_empty(), "{pattern} was interpreted as a pattern");
    }
    // Edges need both ends to match.
    e.set_filters(Filters { search: "orders".into(), ..Filters::default() });
    assert_eq!(e.visible_state().summary.visible_edges, 0);
    e.set_filters(Filters { search: "r".into(), ..Filters::default() });
    let vs = e.visible_state();
    assert_eq!(
        visible_edge_pairs(&e, &vs),
        BTreeSet::from([("svc_orders".to_owned(), "res_db".to_owned())]),
        "gateway has no 'r'"
    );
}

#[test]
fn hide_inferred_drops_inferred_nodes_and_edges() {
    let mut e = system();
    e.seek(BASE + 5 * MS, 1);
    e.set_filters(Filters { hide_inferred: true, ..Filters::default() });
    let vs = e.visible_state();
    assert!(!visible_ids(&e, &vs).contains("res_db"));
    assert_eq!(visible_edge_pairs(&e, &vs), BTreeSet::from([("svc_gateway".to_owned(), "svc_orders".to_owned())]));
    for c in vs.node_data.chunks(NODE_STRIDE).chain(vs.edge_data.chunks(EDGE_STRIDE)) {
        assert_eq!(*c.last().unwrap() as u32 & FLAG_INFERRED, 0);
    }
}

#[test]
fn only_problems_keeps_problem_nodes_and_edges_between_them() {
    let mut e = system();
    e.seek(BASE + 9 * MS, 1);
    e.set_filters(Filters { only_problems: true, ..Filters::default() });
    let vs = e.visible_state();
    // orders failed its only request; postgres saw one failing call; gateway and billing are healthy.
    assert_eq!(visible_ids(&e, &vs), BTreeSet::from(["svc_orders".to_owned(), "res_db".to_owned()]));
    for c in vs.node_data.chunks(NODE_STRIDE) {
        assert_eq!(c[6] as u32 & FLAG_PROBLEM, FLAG_PROBLEM);
    }
    assert_eq!(
        visible_edge_pairs(&e, &vs),
        BTreeSet::from([("svc_orders".to_owned(), "res_db".to_owned())]),
        "no edge may lead to a node hidden by a filter"
    );
}

#[test]
fn filters_combine() {
    let mut e = system();
    e.seek(BASE + 9 * MS, 1);
    e.set_filters(Filters { only_problems: true, hide_inferred: true, ..Filters::default() });
    let vs = e.visible_state();
    assert_eq!(visible_ids(&e, &vs), BTreeSet::from(["svc_orders".to_owned()]));
    assert_eq!(vs.summary.visible_edges, 0);
    e.set_filters(Filters { only_problems: true, search: "post".into(), ..Filters::default() });
    let vs = e.visible_state();
    assert_eq!(visible_ids(&e, &vs), BTreeSet::from(["res_db".to_owned()]));
    e.set_filters(Filters { search: "svc".into(), hide_inferred: true, only_problems: true, ..Filters::default() });
    assert_eq!(e.visible_state().summary.visible_nodes, 1);
    // Filters never change the summary aggregates, only what is drawn.
    let full = {
        e.set_filters(Filters::default());
        e.visible_state().summary
    };
    e.set_filters(Filters { search: "nothing".into(), ..Filters::default() });
    let filtered = e.visible_state().summary;
    assert_eq!(
        (filtered.total_active, filtered.window_requests, filtered.window_errors),
        (full.total_active, full.window_requests, full.window_errors)
    );
}

#[test]
fn trace_selection_only_highlights_that_trace() {
    let mut e = system();
    e.seek(BASE + 5 * MS, 1);
    e.set_filters(Filters { selected_trace_id: Some("b0".into()), ..Filters::default() });
    let vs = e.visible_state();
    let billing = node_idx(&e, "svc_billing");
    for (k, &i) in vs.node_indices.iter().enumerate() {
        let hl = vs.node_data[k * NODE_STRIDE + 6] as u32 & FLAG_HIGHLIGHT != 0;
        assert_eq!(hl, i == billing, "node {i}");
    }
    assert!(vs.edge_data.chunks(EDGE_STRIDE).all(|c| c[7] as u32 & FLAG_HIGHLIGHT == 0));
    assert!(vs.hop_data.chunks(HOP_STRIDE).all(|h| h[5] as u32 & HOP_HIGHLIGHT == 0));

    e.set_filters(Filters { selected_trace_id: Some("no-such-trace".into()), ..Filters::default() });
    let vs = e.visible_state();
    assert!(vs.node_data.chunks(NODE_STRIDE).all(|c| c[6] as u32 & FLAG_HIGHLIGHT == 0));

    e.set_filters(Filters { selected_trace_id: Some("t1".into()), ..Filters::default() });
    let vs = e.visible_state();
    assert!(vs.edge_data.chunks(EDGE_STRIDE).all(|c| c[7] as u32 & FLAG_HIGHLIGHT != 0), "whole t1 path");
    assert!(!vs.hop_data.is_empty() && vs.hop_data.chunks(HOP_STRIDE).all(|h| h[5] as u32 & HOP_HIGHLIGHT != 0));
}

#[test]
fn muting_rules() {
    let mut e = system();
    e.seek(BASE + 5 * MS, 1);
    let dimmed = |e: &mut Engine, muted: &[&str]| -> BTreeSet<String> {
        e.set_filters(Filters {
            muted_services: muted.iter().map(|s| (*s).to_owned()).collect(),
            ..Filters::default()
        });
        let vs = e.visible_state();
        let topo = e.topology();
        vs.node_indices
            .iter()
            .zip(vs.node_data.chunks(NODE_STRIDE))
            .filter(|(_, c)| c[6] as u32 & FLAG_DIMMED != 0)
            .map(|(&i, _)| topo.nodes[i as usize].id.clone())
            .collect()
    };
    assert!(dimmed(&mut e, &["svc_unknown"]).is_empty());
    // A resource id is not a service: muting it directly does nothing.
    assert!(dimmed(&mut e, &["res_db"]).is_empty());
    // The only caller of postgres is orders: muting orders dims postgres too.
    assert_eq!(dimmed(&mut e, &["svc_orders"]), BTreeSet::from(["svc_orders".to_owned(), "res_db".to_owned()]));
    // Duplicated entries behave like one.
    assert_eq!(dimmed(&mut e, &["svc_billing", "svc_billing"]), BTreeSet::from(["svc_billing".to_owned()]));
    // Muting never hides anything.
    e.set_filters(Filters { muted_services: vec!["svc_gateway".into(), "svc_orders".into()], ..Filters::default() });
    assert_eq!(e.visible_state().summary.visible_nodes, 4);
}

#[test]
fn uncalled_resources_are_never_dimmed() {
    let mut e = Engine::new(EngineConfig::default());
    e.load_topology(db_node());
    e.apply_batch(Batch { items: vec![TimelineItem::Span(span("t", "a", None, "api", 2, 0, SEC, false))] }).unwrap();
    e.set_filters(Filters { muted_services: vec!["svc_api".into()], ..Filters::default() });
    let d = e.dimmed_nodes();
    assert!(!d[node_idx(&e, "res_db") as usize]);
    assert!(d[node_idx(&e, "svc_api") as usize]);
}

// ------------------------------------------------------------------ pick, compare, metrics

#[test]
fn pick_hits_nodes_and_misses_empty_space() {
    let mut e = system();
    e.seek(BASE, 1);
    let topo = e.topology();
    for n in &topo.nodes {
        assert_eq!(e.pick(n.x, n.y).as_deref(), Some(n.id.as_str()), "center of {}", n.id);
        assert_eq!(e.pick(n.x + 29.0, n.y).as_deref(), Some(n.id.as_str()), "edge of {}", n.id);
    }
    let max_x = topo.nodes.iter().map(|n| n.x).fold(f32::MIN, f32::max);
    assert_eq!(e.pick(max_x + 1_000.0, 0.0), None);
    assert_eq!(e.pick(f32::NAN, 0.0), None);
    assert_eq!(e.pick(f32::INFINITY, f32::NEG_INFINITY), None);
    assert_eq!(Engine::new(EngineConfig::default()).pick(0.0, 0.0), None);
}

#[test]
fn compare_is_symmetric_and_empty_for_equal_times() {
    let mut e = system();
    assert!(e.compare(BASE + 5 * MS, BASE + 5 * MS).is_empty());
    let ab = e.compare(BASE - SEC, BASE + 9 * MS);
    let ba = e.compare(BASE + 9 * MS, BASE - SEC);
    assert!(!ab.is_empty());
    assert_eq!(ab.len(), ba.len());
    for (x, y) in ab.iter().zip(&ba) {
        assert_eq!((&x.id, &x.before, &x.after), (&y.id, &y.after, &y.before));
    }
    // Every reported node really differs; no-data times report nothing.
    assert!(ab.iter().all(|d| d.before != d.after));
    assert!(e.compare(BASE - 10 * SEC, BASE - 5 * SEC).is_empty());
    // compare does not move the selected time.
    e.seek(BASE + MS, 1);
    e.compare(BASE, BASE + 9 * MS);
    assert_eq!(e.current_time(), Some(BASE + MS));
}

#[test]
fn node_metrics_edge_cases() {
    let mut e = system();
    assert_eq!(e.node_metrics("svc_nope", Some(BASE)), None);
    // Without a selected time and no explicit time the state is empty.
    let m = e.node_metrics("svc_orders", None).unwrap();
    assert_eq!((m.status, m.window_requests), (HealthStatusDto::Unknown, 0));
    // Resources derive status from callers and never report a latency.
    let db = e.node_metrics("res_db", Some(BASE + 9 * MS)).unwrap();
    assert_eq!((db.window_requests, db.window_errors, db.status), (1, 1, HealthStatusDto::Failing));
    assert_eq!(db.avg_latency_ms, None);
    assert!(db.status_reason.contains("not instrumented"));
    let idle = e.node_metrics("res_db", Some(BASE - SEC)).unwrap();
    assert_eq!(idle.status, HealthStatusDto::Unknown);
    let billing = e.node_metrics("svc_billing", Some(BASE + 9 * MS)).unwrap();
    assert_eq!(billing.avg_latency_ms, Some(1.0));
    assert_eq!(billing.status, HealthStatusDto::Healthy);
}

#[test]
fn error_logs_alone_degrade_and_in_flight_is_healthy() {
    let mut e = Engine::new(EngineConfig::default());
    e.apply_batch(Batch {
        items: vec![
            TimelineItem::Log(log("l1", "a", 0, 17)),
            TimelineItem::Log(log("l2", "b", 0, 9)),
            TimelineItem::Span(span("t", "s", None, "c", 2, 0, 10 * SEC, false)),
        ],
    })
    .unwrap();
    let at = Some(BASE + SEC);
    assert_eq!(e.node_metrics("svc_a", at).unwrap().status, HealthStatusDto::Degraded);
    let b = e.node_metrics("svc_b", at).unwrap();
    assert_eq!(b.status, HealthStatusDto::Unknown);
    assert!(b.status_reason.contains("activity"));
    let c = e.node_metrics("svc_c", at).unwrap();
    assert_eq!((c.status, c.active), (HealthStatusDto::Healthy, 1));
}

// ------------------------------------------------------------------ frames, seeks, generations

#[test]
fn changed_nodes_report_status_transitions_only() {
    let mut e = system();
    e.seek(BASE - SEC, 1);
    let first = e.visible_state();
    assert_eq!(first.changed_nodes.len(), e.topology().nodes.len(), "first frame reports every node");
    assert!(e.visible_state().changed_nodes.is_empty(), "same frame again: nothing changed");
    e.seek(BASE + 9 * MS, 2);
    let changed: BTreeSet<u32> = e.visible_state().changed_nodes.into_iter().collect();
    assert!(changed.contains(&node_idx(&e, "svc_orders")));
    // A new node resizes the status vector: every node is reported again.
    e.apply_batch(Batch { items: vec![TimelineItem::Log(log("x", "new", 0, 9))] }).unwrap();
    e.seek(BASE + 9 * MS, 3);
    assert_eq!(e.visible_state().changed_nodes.len(), e.topology().nodes.len());
}

#[test]
fn generations_order_seeks() {
    let mut e = system();
    assert_eq!(e.seek(BASE + MS, 5), SeekOutcome::Applied);
    assert_eq!(e.seek(BASE + 2 * MS, 5), SeekOutcome::Applied, "equal generation is not stale");
    assert_eq!(e.seek(BASE, 4), SeekOutcome::Stale);
    assert_eq!(e.seek(BASE, 0), SeekOutcome::Stale);
    let vs = e.visible_state();
    assert_eq!((vs.summary.generation, vs.summary.time_ns), (5, Some(BASE + 2 * MS)));
    assert_eq!(e.diagnostics().stale_seeks_ignored, 2);
    assert_eq!(e.seek(BASE, u64::MAX), SeekOutcome::Applied);
    assert_eq!(e.seek(BASE + SEC, u64::MAX - 1), SeekOutcome::Stale);
    assert_eq!(e.current_time(), Some(BASE));
}

#[test]
fn frame_without_seek_is_empty_but_valid() {
    let mut e = system();
    let vs = e.visible_state();
    check_frame(&e, &vs).unwrap();
    assert_eq!(vs.summary.time_ns, None);
    assert!(vs.hop_data.is_empty());
    assert_eq!((vs.summary.total_active, vs.summary.window_requests), (0, 0));
    assert_eq!(vs.summary.visible_nodes, 4);
}

#[test]
fn viewport_lod_and_degenerate_viewports() {
    let mut e = system();
    e.seek(BASE + 5 * MS, 1);
    let lod = |e: &mut Engine, scale: f32| {
        e.set_viewport(Viewport { x: -1e6, y: -1e6, width: 2e6, height: 2e6, scale });
        e.visible_state().summary.lod
    };
    assert_eq!((lod(&mut e, 0.1), lod(&mut e, 1.0), lod(&mut e, 3.0), lod(&mut e, f32::NAN)), (0, 1, 2, 1));
    // Zero or negative sizes mean "not measured yet": nothing is culled.
    e.set_viewport(Viewport { x: 1e9, y: 1e9, width: 0.0, height: 10.0, scale: 1.0 });
    assert_eq!(e.visible_state().summary.visible_nodes, 4);
    e.set_viewport(Viewport { x: f32::NAN, y: f32::NAN, width: f32::NAN, height: f32::NAN, scale: f32::NAN });
    frame(&mut e).unwrap();
}

#[test]
fn applying_the_same_batch_twice_is_idempotent() {
    let batch: Batch = serde_json::from_str(&valid_batch_json()).unwrap();
    let mut once = Engine::new(EngineConfig::default());
    once.apply_batch(batch.clone()).unwrap();
    let mut twice = Engine::new(EngineConfig::default());
    twice.apply_batch(batch.clone()).unwrap();
    twice.seek(BASE, 1);
    twice.visible_state();
    twice.apply_batch(batch.clone()).unwrap();
    once.seek(BASE + SEC, 2);
    twice.seek(BASE + SEC, 2);
    let (a, b) = (once.visible_state(), twice.visible_state());
    assert_eq!((a.node_data, a.edge_data, a.hop_data, a.summary), (b.node_data, b.edge_data, b.hop_data, b.summary));
    let d = twice.diagnostics();
    assert_eq!(d.duplicates_ignored, batch.items.len() as u64);
    assert_eq!(d.timeline_rebuilds, 1, "identical duplicates change nothing");
    assert_eq!(twice.item_count(), once.item_count());
}

#[test]
fn a_changed_duplicate_replaces_the_item() {
    let mut e = Engine::new(EngineConfig::default());
    e.apply_batch(Batch { items: vec![TimelineItem::Span(span("t", "s", None, "api", 2, 0, SEC, false))] }).unwrap();
    assert_eq!(e.node_metrics("svc_api", Some(BASE + SEC)).unwrap().window_errors, 0);
    e.apply_batch(Batch { items: vec![TimelineItem::Span(span("t", "s", None, "api", 2, 0, SEC, true))] }).unwrap();
    let m = e.node_metrics("svc_api", Some(BASE + SEC)).unwrap();
    assert_eq!((m.window_requests, m.window_errors), (1, 1));
    e.apply_batch(Batch { items: vec![TimelineItem::Log(log("l", "api", 0, 9))] }).unwrap();
    e.apply_batch(Batch { items: vec![TimelineItem::Log(log("l", "api", 0, 17))] }).unwrap();
    e.apply_batch(Batch { items: vec![TimelineItem::Event(event("e", Some("api"), "a", 0))] }).unwrap();
    e.apply_batch(Batch { items: vec![TimelineItem::Event(event("e", Some("api"), "a", SEC * 100))] }).unwrap();
    let m = e.node_metrics("svc_api", Some(BASE + SEC)).unwrap();
    assert_eq!((m.window_logs, m.window_error_logs, m.window_events), (1, 1, 0));
    assert_eq!(e.item_count(), 3);
}

// ------------------------------------------------------------------ eviction

#[test]
fn evict_before_is_monotonic_and_counts() {
    let mut e = system();
    e.seek(BASE + 5 * MS, 1);
    let n = e.item_count();
    e.evict_before(BASE - SEC);
    assert_eq!(e.diagnostics().evicted_items, 0);
    assert_eq!(e.diagnostics().evicted_before_ns, None, "nothing evicted: no bound recorded");
    e.evict_before(BASE + 3 * MS);
    let d = e.diagnostics();
    // billing b0..b2 end at 1..3 ms; b2 ends at exactly 3 ms and is kept.
    assert_eq!(d.evicted_items, 2);
    assert_eq!(e.item_count(), n - 2);
    assert_eq!(d.evicted_before_ns, Some(BASE + 3 * MS));
    e.evict_before(BASE + 4 * MS);
    e.evict_before(BASE + MS);
    assert_eq!(e.diagnostics().evicted_before_ns, Some(BASE + 4 * MS), "the bound never moves back");
    // A span that started before the cutoff but ended after it is kept whole: active never goes negative.
    let g = e.node_metrics("svc_gateway", Some(BASE + 5 * MS)).unwrap();
    assert_eq!(g.active, 1);
}

#[test]
fn max_items_zero_keeps_nothing() {
    let mut e = Engine::new(EngineConfig { max_items: 0, ..EngineConfig::default() });
    e.apply_batch(Batch {
        items: vec![TimelineItem::Log(log("a", "x", 0, 9)), TimelineItem::Log(log("b", "x", 1, 9))],
    })
    .unwrap();
    assert_eq!(e.item_count(), 0);
    assert_eq!(e.diagnostics().evicted_items, 2);
    e.seek(BASE, 1);
    frame(&mut e).unwrap();
}

// ------------------------------------------------------------------ oracles

#[derive(Debug, Clone)]
enum Op {
    Apply(usize),
    Seek(i64),
    Evict(i64),
}

fn arb_world() -> impl Strategy<Value = Vec<TimelineItem>> {
    let svc = prop::sample::select(vec!["a", "b", "c", "d"]);
    let spans = prop::collection::vec(
        (0u32..30, svc.clone(), 0i64..100, 0i64..30, any::<bool>(), 0u8..6, any::<bool>()),
        0..50,
    )
    .prop_map(|v| {
        v.into_iter()
            .map(|(id, s, start, dur, err, kind, db)| {
                let parent = if id % 4 == 0 { None } else { Some(format!("s{}", id / 2)) };
                let mut x = span(
                    &format!("t{}", id % 5),
                    &format!("s{id}"),
                    parent.as_deref(),
                    s,
                    kind,
                    start * SEC,
                    (start + dur) * SEC,
                    err,
                );
                if db && kind == 3 {
                    x.target_entity_id = Some("res_db".into());
                }
                TimelineItem::Span(x)
            })
            .collect::<Vec<_>>()
    });
    let logs =
        prop::collection::vec((0u32..30, svc.clone(), 0i64..130, prop::sample::select(vec![9u8, 13, 17, 21])), 0..40)
            .prop_map(|v| {
                v.into_iter()
                    .map(|(id, s, t, sev)| TimelineItem::Log(log(&format!("l{id}"), s, t * SEC, sev)))
                    .collect::<Vec<_>>()
            });
    let events = prop::collection::vec((0u32..20, prop::option::of(svc), 0i64..130), 0..20).prop_map(|v| {
        v.into_iter()
            .map(|(id, s, t)| TimelineItem::Event(event(&format!("e{id}"), s, "deploy", t * SEC)))
            .collect::<Vec<_>>()
    });
    (spans, logs, events).prop_map(|(mut a, b, c)| {
        a.extend(b);
        a.extend(c);
        a
    })
}

fn arb_ops() -> impl Strategy<Value = Vec<Op>> {
    let op = prop_oneof![
        3 => (1usize..20).prop_map(Op::Apply),
        4 => (-5i64..140).prop_map(Op::Seek),
        1 => (0i64..80).prop_map(Op::Evict),
    ];
    prop::collection::vec(op, 1..16)
}

/// The engine's recorded set, maintained independently: last write wins per ID, eviction drops
/// spans ending (and logs/events happening) before the cutoff.
#[derive(Default)]
struct Model {
    spans: BTreeMap<(String, String), SpanItem>,
    logs: BTreeMap<String, LogItem>,
    events: BTreeMap<String, EventItem>,
}

impl Model {
    fn apply(&mut self, items: &[TimelineItem]) {
        for i in items {
            match i {
                TimelineItem::Span(s) => {
                    self.spans.insert((s.trace_id.clone(), s.span_id.clone()), s.clone());
                }
                TimelineItem::Log(l) => {
                    self.logs.insert(l.id.clone(), l.clone());
                }
                TimelineItem::Event(e) => {
                    self.events.insert(e.id.clone(), e.clone());
                }
            }
        }
    }

    fn evict(&mut self, before: i64) {
        self.spans.retain(|_, s| s.end_ns >= before);
        self.logs.retain(|_, l| l.time_ns >= before);
        self.events.retain(|_, e| e.time_ns >= before);
    }

    fn check(&self, e: &mut Engine, t: i64, window: i64) -> Result<(), TestCaseError> {
        let spans: Vec<SpanItem> = self.spans.values().cloned().collect();
        let logs: Vec<LogItem> = self.logs.values().cloned().collect();
        let expected = reference::metrics_at(&spans, &logs, t, window);
        let services: Vec<String> =
            e.topology().nodes.iter().filter(|n| n.kind == EntityKindDto::Service).map(|n| n.id.clone()).collect();
        for svc in services {
            let m = expected.get(&svc).cloned().unwrap_or_default();
            let got = e.node_metrics(&svc, Some(t)).unwrap();
            let got_tuple = [
                got.active,
                got.window_requests,
                got.window_completed,
                got.window_errors,
                got.window_logs,
                got.window_error_logs,
            ]
            .map(i64::from);
            prop_assert_eq!(
                got_tuple,
                [m.active, m.started, m.completed, m.errors, m.logs, m.error_logs],
                "{} at {}",
                svc,
                t
            );
            let events = self
                .events
                .values()
                .filter(|x| {
                    x.service_id.as_deref() == Some(svc.as_str())
                        && x.time_ns > t.saturating_sub(window)
                        && x.time_ns <= t
                })
                .count();
            prop_assert_eq!(got.window_events as usize, events, "events of {}", svc);
        }
        Ok(())
    }
}

fn edge_metrics(e: &mut Engine, t: i64) -> BTreeMap<(String, String), (u32, u32, u32)> {
    e.seek(t, u64::MAX);
    e.set_filters(Filters::default());
    e.viewport = None;
    let vs = e.visible_state();
    let topo = e.topology();
    vs.edge_indices
        .iter()
        .zip(vs.edge_data.chunks(EDGE_STRIDE))
        .map(|(&i, c)| {
            let x = &topo.edges[i as usize];
            let key = (topo.nodes[x.source as usize].id.clone(), topo.nodes[x.target as usize].id.clone());
            (key, (c[4] as u32, c[5] as u32, (c[6] * 1000.0).round() as u32))
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Any interleaving of live batches (with duplicates and out-of-order items), seeks
    /// backwards and forwards, and evictions agrees with the naive reference at every seek.
    #[test]
    fn live_session_matches_reference(items in arb_world(), ops in arb_ops(), small_budget in any::<bool>(), dup in any::<bool>()) {
        let window = 20 * SEC;
        let cfg = EngineConfig {
            rolling_window_ns: window,
            checkpoint_budget_bytes: if small_budget { 1 } else { 16 << 20 },
            ..EngineConfig::default()
        };
        let mut e = Engine::new(cfg);
        e.load_topology(db_node());
        let mut model = Model::default();
        let mut cursor = 0usize;
        let mut generation = 0u64;
        for op in ops {
            match op {
                Op::Apply(n) => {
                    let end = (cursor + n).min(items.len());
                    let mut chunk = items[cursor..end].to_vec();
                    if dup && cursor > 0 {
                        // Re-deliver an earlier item; the model applies it in the same order
                        // (last write wins, evicted items come back as new).
                        chunk.push(items[cursor / 2].clone());
                    }
                    model.apply(&chunk);
                    e.apply_batch(Batch { items: chunk }).unwrap();
                    cursor = end;
                }
                Op::Seek(t) => {
                    generation += 1;
                    let tt = BASE + t * SEC;
                    prop_assert_eq!(e.seek(tt, generation), SeekOutcome::Applied);
                    model.check(&mut e, tt, window)?;
                    frame(&mut e)?;
                }
                Op::Evict(c) => {
                    e.evict_before(BASE + c * SEC);
                    model.evict(BASE + c * SEC);
                }
            }
        }
        let d = e.diagnostics();
        prop_assert_eq!(d.spans as usize, model.spans.len());
        prop_assert_eq!(d.logs as usize, model.logs.len());
        prop_assert_eq!(d.events as usize, model.events.len());
    }

    /// Incrementally merged timelines (resources, events, late parents) equal a full rebuild,
    /// edge by edge, at any time.
    #[test]
    fn live_merge_equals_rebuild(items in arb_world(), chunks in 1usize..6, times in prop::collection::vec(-5i64..140, 1..5)) {
        // Edges discovered from a span stay when the span is later replaced by different content
        // under the same ID, so only identical re-deliveries are order-independent.
        let mut model = Model::default();
        model.apply(&items);
        let mut items: Vec<TimelineItem> = model.spans.into_values().map(TimelineItem::Span)
            .chain(model.logs.into_values().map(TimelineItem::Log))
            .chain(model.events.into_values().map(TimelineItem::Event))
            .collect();
        items.reverse();
        let again: Vec<TimelineItem> = items.iter().step_by(2).cloned().collect();
        items.extend(again);
        let mut inc = Engine::new(EngineConfig::default());
        inc.load_topology(db_node());
        let size = items.len().div_ceil(chunks).max(1);
        for (g, chunk) in items.chunks(size).enumerate() {
            inc.apply_batch(Batch { items: chunk.to_vec() }).unwrap();
            inc.seek(BASE + times[g % times.len()] * SEC, g as u64 + 1);
            inc.visible_state();
        }
        let mut full = Engine::new(EngineConfig::default());
        full.load_topology(db_node());
        full.apply_batch(Batch { items: items.clone() }).unwrap();
        for t in times {
            let tt = BASE + t * SEC;
            prop_assert_eq!(edge_metrics(&mut inc, tt), edge_metrics(&mut full, tt), "edges at {}", t);
            for n in full.topology().nodes {
                prop_assert_eq!(inc.node_metrics(&n.id, Some(tt)), full.node_metrics(&n.id, Some(tt)), "node {}", n.id);
            }
        }
        prop_assert_eq!(inc.diagnostics().unresolved_parents, full.diagnostics().unresolved_parents);
    }

    /// Frame invariants hold under every filter/viewport combination.
    #[test]
    fn frames_are_well_formed(
        items in arb_world(),
        t in -5i64..140,
        search in prop::sample::select(vec!["", "a", "B", ".*", "svc_", "zz", "post"]),
        hide_inferred in any::<bool>(),
        only_problems in any::<bool>(),
        trace in prop::option::of(prop::sample::select(vec!["t0", "t1", "none"])),
        muted in prop::collection::vec(prop::sample::select(vec!["svc_a", "svc_b", "res_db", "x"]), 0..3),
        viewport in prop::option::of((-2000.0f32..2000.0, -2000.0f32..2000.0, 0.0f32..3000.0, 0.0f32..3000.0, 0.1f32..3.0)),
        max_hops in 0u32..5,
    ) {
        let mut e = Engine::new(EngineConfig { max_hops, ..EngineConfig::default() });
        e.load_topology(db_node());
        e.apply_batch(Batch { items }).unwrap();
        e.set_hop_window(30 * SEC, 30 * SEC);
        let filters = Filters {
            search: search.into(),
            hide_inferred,
            only_problems,
            selected_trace_id: trace.map(Into::into),
            muted_services: muted.into_iter().map(Into::into).collect(),
        };
        e.set_filters(filters.clone());
        if let Some((x, y, w, h, scale)) = viewport {
            e.set_viewport(Viewport { x, y, width: w, height: h, scale });
        }
        e.seek(BASE + t * SEC, 1);
        let vs = e.visible_state();
        check_frame(&e, &vs)?;

        // Every drawn node passes every filter; with no viewport every passing node is drawn.
        let topo = e.topology();
        let needle = filters.search.to_lowercase();
        let passing: BTreeSet<u32> = topo.nodes.iter().filter(|n| {
            let m = e.node_metrics(&n.id, None).unwrap();
            (!filters.hide_inferred || n.provenance != ProvenanceDto::Inferred)
                && (needle.is_empty() || n.name.to_lowercase().contains(&needle) || n.id.contains(&needle))
                && (!filters.only_problems || matches!(m.status, HealthStatusDto::Degraded | HealthStatusDto::Failing))
        }).map(|n| n.index).collect();
        let drawn: BTreeSet<u32> = vs.node_indices.iter().copied().collect();
        prop_assert!(drawn.is_subset(&passing));
        if viewport.is_none() {
            prop_assert_eq!(&drawn, &passing);
        }
        for &i in &vs.edge_indices {
            let x = &topo.edges[i as usize];
            prop_assert!(passing.contains(&x.source) && passing.contains(&x.target), "edge {} to a filtered node", i);
        }
        // Dimmed flags follow `muted_services` exactly.
        let dimmed = e.dimmed_nodes();
        for (k, &i) in vs.node_indices.iter().enumerate() {
            let flag = vs.node_data[k * NODE_STRIDE + 6] as u32 & FLAG_DIMMED != 0;
            prop_assert_eq!(flag, dimmed[i as usize]);
        }
        if filters.muted_services.iter().all(|m| !m.starts_with("svc_")) {
            prop_assert!(dimmed.iter().all(|d| !d));
        }
    }

    /// max_items bounds memory after every batch and nothing is lost uncounted.
    #[test]
    fn max_items_bounds_the_window(items in arb_world(), max in 1u32..40, chunks in 1usize..5) {
        let mut e = Engine::new(EngineConfig { max_items: max, ..EngineConfig::default() });
        let size = items.len().div_ceil(chunks).max(1);
        let mut model = Model::default();
        for (g, chunk) in items.chunks(size).enumerate() {
            e.apply_batch(Batch { items: chunk.to_vec() }).unwrap();
            model.apply(chunk);
            prop_assert!(e.item_count() <= max as usize);
            e.seek(BASE + 50 * SEC, g as u64 + 1);
            frame(&mut e)?;
        }
        // Everything that was ever unique is either stored or counted as evicted. Duplicates of an
        // evicted item re-enter as new, so the count can only be higher.
        let d = e.diagnostics();
        let unique = model.spans.len() + model.logs.len() + model.events.len();
        prop_assert!(e.item_count() as u64 + d.evicted_items >= unique as u64);
        prop_assert_eq!(e.item_count(), (d.spans + d.logs + d.events) as usize);
    }
}
