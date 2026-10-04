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
        source_entity_id: None,
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
fn layout_does_not_move_when_only_items_change() {
    let mut e = engine_with(demo_items());
    e.seek(BASE + SEC, 1);
    let before: Vec<(String, f32, f32)> = e.topology().nodes.iter().map(|n| (n.id.clone(), n.x, n.y)).collect();
    let rev = e.topology().revision;
    e.apply_batch(Batch { items: vec![TimelineItem::Log(log("lx", "orders", 4 * SEC, 9))] }).unwrap();
    e.seek(BASE + 5 * SEC, 2);
    let after = e.topology();
    assert_eq!(after.revision, rev, "no new node or edge: no relayout");
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

// ------------------------------------------------------------------ hops

const MS: i64 = 1_000_000;
const US: i64 = 1_000;

fn db_span(trace: &str, id: &str, parent: &str, start: i64, end: i64, err: bool) -> SpanItem {
    SpanItem { target_entity_id: Some("res_db".into()), ..span(trace, id, Some(parent), "orders", 3, start, end, err) }
}

/// gateway client (t=0..10 ms) -> orders server (1..9 ms) -> db client (2 ms + 5 µs .. 7 ms).
fn chain_items() -> Vec<TimelineItem> {
    vec![
        TimelineItem::Span(span("t1", "g1", None, "gateway", 2, 0, 11 * MS, false)),
        TimelineItem::Span(span("t1", "g2", Some("g1"), "gateway", 3, 0, 10 * MS, false)),
        TimelineItem::Span(span("t1", "o1", Some("g2"), "orders", 2, MS, 9 * MS, false)),
        TimelineItem::Span(db_span("t1", "d1", "o1", 2 * MS + 5 * US, 7 * MS, true)),
    ]
}

fn chain_engine() -> Engine {
    let mut e = Engine::new(EngineConfig::default());
    e.load_topology(TopologySnapshot {
        nodes: vec![TopologyNode {
            id: "res_db".into(),
            name: "postgres".into(),
            kind: EntityKindDto::Database,
            provenance: ProvenanceDto::Inferred,
        }],
        edges: vec![],
    });
    e.apply_batch(Batch { items: chain_items() }).unwrap();
    e
}

fn hops(vs: &VisibleState) -> Vec<&[f32]> {
    vs.hop_data.chunks(HOP_STRIDE).collect()
}

fn edge_between(e: &Engine, from: &str, to: &str) -> u32 {
    let t = e.topology();
    let idx = |id: &str| t.nodes.iter().find(|n| n.id == id).unwrap().index;
    t.edges.iter().find(|x| x.source == idx(from) && x.target == idx(to)).unwrap().index
}

#[test]
fn hops_carry_the_recorded_timestamps_of_each_call() {
    let mut e = chain_engine();
    e.seek(BASE + 5 * MS, 1);
    let vs = e.visible_state();
    let call_edge = edge_between(&e, "svc_gateway", "svc_orders") as f32;
    let db_edge = edge_between(&e, "svc_orders", "res_db") as f32;
    let hs = hops(&vs);
    assert_eq!(hs.len(), 2);
    assert_eq!(vs.summary.hops_in_window, 2);
    assert_eq!(vs.summary.hops_shown, 2);

    // Times are milliseconds relative to the frame time (5 ms).
    let call = hs.iter().find(|h| h[0] == call_edge).unwrap();
    assert_eq!(&call[1..5], &[-5.0, -4.0, 4.0, 5.0], "client start, server start, server end, client end");
    assert_eq!(call[5] as u32, 0);

    let db = hs.iter().find(|h| h[0] == db_edge).unwrap();
    // Leaf: only departure and return are known; microsecond precision survives.
    assert!((db[1] - -2.995).abs() < 1e-4 && db[1] == db[2], "{db:?}");
    assert_eq!((db[3], db[4]), (2.0, 2.0));
    assert_eq!(db[5] as u32, HOP_LEAF | HOP_ERROR);
}

#[test]
fn hop_window_selects_overlapping_calls_only() {
    let mut e = chain_engine();
    e.set_hop_window(MS, 0);
    // At 12 ms everything ended before 11 ms: nothing overlaps [11 ms, 12 ms].
    e.seek(BASE + 12 * MS, 1);
    assert!(e.visible_state().hop_data.is_empty());
    // [9.5 ms, 10.5 ms] still overlaps the gateway -> orders call (ends at 10 ms).
    e.seek(BASE + 10 * MS + MS / 2, 2);
    let vs = e.visible_state();
    assert_eq!(hops(&vs).len(), 1);
    // Before the first departure, with a lookahead, the call shows up ahead of time.
    e.set_hop_window(0, 3 * MS);
    e.seek(BASE - 2 * MS, 3);
    let vs = e.visible_state();
    assert_eq!(hops(&vs).len(), 1);
    assert_eq!(hops(&vs)[0][1], 2.0);
}

#[test]
fn hop_cap_is_reported() {
    let mut e = Engine::new(EngineConfig { max_hops: 1, ..EngineConfig::default() });
    e.load_topology(TopologySnapshot {
        nodes: vec![TopologyNode {
            id: "res_db".into(),
            name: "postgres".into(),
            kind: EntityKindDto::Database,
            provenance: ProvenanceDto::Inferred,
        }],
        edges: vec![],
    });
    e.apply_batch(Batch { items: chain_items() }).unwrap();
    e.seek(BASE + 5 * MS, 1);
    let vs = e.visible_state();
    assert_eq!((vs.summary.hops_in_window, vs.summary.hops_shown), (2, 1));
    assert_eq!(vs.hop_data.len(), HOP_STRIDE);
}

// ------------------------------------------------------------------ external systems

fn ext_node(id: &str, name: &str, kind: EntityKindDto) -> TopologyNode {
    TopologyNode { id: id.into(), name: name.into(), kind, provenance: ProvenanceDto::Inferred }
}

/// Browser -> api (root server span, 0..10 ms) -> api calls a payment provider (2..6 ms, failed).
fn external_items(trace: &str, offset: i64) -> Vec<TimelineItem> {
    vec![
        TimelineItem::Span(SpanItem {
            source_entity_id: Some("ext_clients".into()),
            ..span(trace, "w1", None, "api", 2, offset, offset + 10 * MS, false)
        }),
        TimelineItem::Span(SpanItem {
            target_entity_id: Some("ext_pay".into()),
            ..span(trace, "w2", Some("w1"), "api", 3, offset + 2 * MS, offset + 6 * MS, true)
        }),
    ]
}

fn external_engine() -> Engine {
    let mut e = Engine::new(EngineConfig::default());
    e.load_topology(TopologySnapshot {
        nodes: vec![
            ext_node("ext_clients", "External clients", EntityKindDto::ExternalClient),
            ext_node("ext_pay", "pay.example.test", EntityKindDto::ExternalDependency),
        ],
        edges: vec![],
    });
    e.apply_batch(Batch { items: external_items("t1", 0) }).unwrap();
    e
}

#[test]
fn external_clients_and_dependencies_get_edges_and_request_response_hops() {
    let mut e = external_engine();
    e.seek(BASE + 5 * MS, 1);
    let topo = e.topology();
    let inbound = edge_between(&e, "ext_clients", "svc_api");
    let outbound = edge_between(&e, "svc_api", "ext_pay");
    let edge = |i: u32| topo.edges.iter().find(|x| x.index == i).unwrap();
    assert_eq!((edge(inbound).relation, edge(inbound).provenance), (RelationKindDto::Calls, ProvenanceDto::Inferred));
    assert!(edge(inbound).reasons.iter().any(|r| r.contains("outside the project")), "{:?}", edge(inbound).reasons);
    assert_eq!(edge(outbound).relation, RelationKindDto::DependsOn);

    let vs = e.visible_state();
    let hs = hops(&vs);
    assert_eq!(hs.len(), 2);
    // The request arrives from outside at the server span start and the answer leaves at its end.
    let inb = hs.iter().find(|h| h[0] == inbound as f32).unwrap();
    assert_eq!((&inb[1..5], inb[5] as u32), (&[-10.0, -5.0, 5.0, 10.0][..], 0));
    let api = e.topology().nodes.iter().find(|n| n.id == "svc_api").unwrap().index;
    assert_eq!(inb[6], api as f32, "coloured by the service where the trace starts");
    let out = hs.iter().find(|h| h[0] == outbound as f32).unwrap();
    assert_eq!((&out[1..5], out[5] as u32), (&[-3.0, -3.0, 1.0, 1.0][..], HOP_LEAF | HOP_ERROR));
}

#[test]
fn an_inbound_request_arrives_before_its_service_calls_anything() {
    // Regression: the inbound hop from an external client was drawn as a leaf, so its dot
    // reached the api halfway through the server span (0 ms here), after the api's own call
    // to the payment provider had already left (-3 ms).
    let mut e = external_engine();
    e.seek(BASE + 5 * MS, 1);
    let vs = e.visible_state();
    let hs = hops(&vs);
    let inb = hs.iter().find(|h| h[0] == edge_between(&e, "ext_clients", "svc_api") as f32).unwrap();
    let out = hs.iter().find(|h| h[0] == edge_between(&e, "svc_api", "ext_pay") as f32).unwrap();
    assert_eq!(inb[5] as u32 & HOP_LEAF, 0, "the callee is instrumented: arrival and answer are measured");
    assert!(inb[2] <= out[1], "arrives at {} but the api calls out at {}", inb[2], out[1]);
    assert!(inb[3] >= out[4], "answers at {} before the api's call is back at {}", inb[3], out[4]);
}

#[test]
fn external_nodes_and_their_edges_are_flagged() {
    let mut e = external_engine();
    e.seek(BASE + 5 * MS, 1);
    let vs = e.visible_state();
    assert_eq!(node_flags(&e, &vs, "ext_clients") & FLAG_EXTERNAL, FLAG_EXTERNAL);
    assert_eq!(node_flags(&e, &vs, "ext_pay") & FLAG_EXTERNAL, FLAG_EXTERNAL);
    assert_eq!(node_flags(&e, &vs, "svc_api") & FLAG_EXTERNAL, 0);
    assert_eq!(edge_flags(&vs, edge_between(&e, "ext_clients", "svc_api")) & FLAG_EXTERNAL, FLAG_EXTERNAL);
    assert_eq!(edge_flags(&vs, edge_between(&e, "svc_api", "ext_pay")) & FLAG_EXTERNAL, FLAG_EXTERNAL);
}

#[test]
fn external_client_metrics_come_from_the_requests_it_sent() {
    let mut e = external_engine();
    e.seek(BASE + 11 * MS, 1);
    let m = e.node_metrics("ext_clients", None).unwrap();
    assert_eq!((m.window_requests, m.window_errors, m.status), (1, 0, HealthStatusDto::Healthy));
    assert!(m.status_reason.contains("services it calls"), "{}", m.status_reason);
    let pay = e.node_metrics("ext_pay", None).unwrap();
    assert_eq!((pay.window_requests, pay.window_errors), (1, 1));

    // Totals do not count the same entry request twice (once for the client, once for the api).
    let mut plain = engine_with(external_items("t1", 0));
    plain.load_topology(TopologySnapshot {
        nodes: vec![ext_node("ext_pay", "pay.example.test", EntityKindDto::ExternalDependency)],
        edges: vec![],
    });
    plain.seek(BASE + 11 * MS, 1);
    let (with, without) = (e.visible_state().summary, plain.visible_state().summary);
    assert_eq!((with.window_requests, with.window_errors), (without.window_requests, without.window_errors));
}

#[test]
fn selecting_a_trace_highlights_its_external_client() {
    let mut e = external_engine();
    e.set_filters(Filters { selected_trace_id: Some("t1".into()), ..Filters::default() });
    e.seek(BASE + 5 * MS, 1);
    let vs = e.visible_state();
    assert_eq!(node_flags(&e, &vs, "ext_clients") & FLAG_HIGHLIGHT, FLAG_HIGHLIGHT);
    assert_eq!(edge_flags(&vs, edge_between(&e, "ext_clients", "svc_api")) & FLAG_HIGHLIGHT, FLAG_HIGHLIGHT);
}

#[test]
fn source_is_ignored_unless_it_is_a_known_node_on_a_root_server_span() {
    let items = vec![
        // Unknown node: nothing is invented.
        TimelineItem::Span(SpanItem {
            source_entity_id: Some("not_in_topology".into()),
            ..span("t1", "a", None, "api", 2, 0, MS, false)
        }),
        // Has an instrumented parent: the caller is the parent's service, not an external client.
        TimelineItem::Span(span("t2", "p", None, "gateway", 3, 0, 2 * MS, false)),
        TimelineItem::Span(SpanItem {
            source_entity_id: Some("ext_clients".into()),
            ..span("t2", "c", Some("p"), "api", 2, 0, MS, false)
        }),
        // Not a server span.
        TimelineItem::Span(SpanItem {
            source_entity_id: Some("ext_clients".into()),
            ..span("t3", "i", None, "api", 1, 0, MS, false)
        }),
    ];
    let mut e = Engine::new(EngineConfig::default());
    e.load_topology(TopologySnapshot {
        nodes: vec![ext_node("ext_clients", "External clients", EntityKindDto::ExternalClient)],
        edges: vec![],
    });
    e.apply_batch(Batch { items }).unwrap();
    e.seek(BASE + MS / 2, 1);
    let topo = e.topology();
    let clients = topo.nodes.iter().find(|n| n.id == "ext_clients").unwrap().index;
    assert!(topo.edges.iter().all(|x| x.source != clients), "{:?}", topo.edges);
    assert_eq!(topo.edges.len(), 1, "only gateway -> api");
}

#[test]
fn live_requests_from_a_known_external_client_merge_without_rebuilds() {
    let mut e = external_engine();
    e.seek(BASE + 10 * MS, 1);
    assert_eq!(e.diagnostics().timeline_rebuilds, 1);
    for i in 1..=3i64 {
        e.apply_batch(Batch { items: external_items(&format!("t{}", i + 1), i * 20 * MS) }).unwrap();
        e.seek(BASE + (i * 20 + 10) * MS, i as u64 + 1);
    }
    assert_eq!(e.diagnostics().timeline_rebuilds, 1, "edges to external nodes are already known");
    assert_eq!(e.diagnostics().timeline_merges, 3);
}

#[test]
fn clock_skew_is_clamped_and_flagged() {
    // The callee's clock runs 2 ms behind: its server span starts before the client span.
    let items = vec![
        TimelineItem::Span(span("t1", "g2", None, "gateway", 3, 3 * MS, 10 * MS, false)),
        TimelineItem::Span(span("t1", "o1", Some("g2"), "orders", 2, MS, 8 * MS, false)),
    ];
    let mut e = engine_with(items);
    e.seek(BASE + 5 * MS, 1);
    let vs = e.visible_state();
    let h = hops(&vs)[0];
    assert!(h[1] <= h[2] && h[2] <= h[3] && h[3] <= h[4], "{h:?}");
    assert_eq!(h[5] as u32 & HOP_CLAMPED, HOP_CLAMPED);
}

#[test]
fn clock_skew_shifts_the_callee_and_its_subtree_for_display() {
    // orders reports its 6 ms call at 1..7 ms inside the gateway's 3..10 ms client span, and
    // payments reports 7..9 ms for a call that orders made at 2..5 ms of its own clock.
    let items = vec![
        TimelineItem::Span(span("t1", "g2", None, "gateway", 3, 3 * MS, 10 * MS, false)),
        TimelineItem::Span(span("t1", "o1", Some("g2"), "orders", 2, MS, 7 * MS, false)),
        TimelineItem::Span(span("t1", "o2", Some("o1"), "orders", 3, 2 * MS, 5 * MS, false)),
        TimelineItem::Span(span("t1", "p1", Some("o2"), "payments", 2, 7 * MS, 9 * MS, false)),
    ];
    let e = engine_with(items);
    let t = e.timeline_tracks(BASE, BASE + 12 * MS, 12, 100);
    let clip = |service: &str, start: f64| {
        let (i, _) = track(&t, service);
        t.clips.iter().any(|c| c.track == i as u32 && (c.start_ms - start).abs() < 1e-9)
    };
    // orders: +2.5 ms (o1 centred at 3.5..9.5), and o2 moves with it to 4.5..7.5.
    assert!(clip("orders", 3.5) && clip("orders", 4.5), "{:?}", t.clips);
    // payments is centred in the shifted o2: 7..9 -> 5..7 (-2 ms).
    assert!(clip("payments", 5.0), "{:?}", t.clips);
    // gateway (the root) is never moved.
    assert!(clip("gateway", 3.0));
}

fn muted(ids: &[&str]) -> Filters {
    Filters { muted_services: ids.iter().map(|s| (*s).to_owned()).collect(), ..Filters::default() }
}

fn node_flags(e: &Engine, vs: &VisibleState, id: &str) -> u32 {
    let idx = e.topology().nodes.iter().find(|n| n.id == id).unwrap().index;
    let k = vs.node_indices.iter().position(|&i| i == idx).unwrap();
    vs.node_data[k * NODE_STRIDE + 6] as u32
}

fn edge_flags(vs: &VisibleState, edge: u32) -> u32 {
    let k = vs.edge_indices.iter().position(|&i| i == edge).unwrap();
    vs.edge_data[k * EDGE_STRIDE + 7] as u32
}

#[test]
fn muted_services_are_dimmed_with_their_edges_and_hops() {
    let mut e = chain_engine();
    e.seek(BASE + 5 * MS, 1);
    e.set_filters(muted(&["svc_gateway"]));
    let vs = e.visible_state();
    let call = edge_between(&e, "svc_gateway", "svc_orders");
    let db = edge_between(&e, "svc_orders", "res_db");
    assert_eq!(node_flags(&e, &vs, "svc_gateway") & FLAG_DIMMED, FLAG_DIMMED);
    assert_eq!(node_flags(&e, &vs, "svc_orders") & FLAG_DIMMED, 0);
    // The database is still called by a live service.
    assert_eq!(node_flags(&e, &vs, "res_db") & FLAG_DIMMED, 0);
    assert_eq!(edge_flags(&vs, call) & FLAG_DIMMED, FLAG_DIMMED);
    assert_eq!(edge_flags(&vs, db) & FLAG_DIMMED, 0);
    for h in hops(&vs) {
        let dim = h[5] as u32 & HOP_DIMMED != 0;
        assert_eq!(dim, h[0] as u32 == call, "{h:?}");
    }

    // With every caller muted, the dependency is dimmed too.
    e.set_filters(muted(&["svc_gateway", "svc_orders"]));
    let vs = e.visible_state();
    assert_eq!(node_flags(&e, &vs, "res_db") & FLAG_DIMMED, FLAG_DIMMED);
    // Nothing muted: nothing dimmed.
    e.set_filters(Filters::default());
    let vs = e.visible_state();
    assert!(vs.node_data.chunks(NODE_STRIDE).all(|n| n[6] as u32 & FLAG_DIMMED == 0));
}

#[test]
fn muted_services_keep_their_row_but_leave_the_aggregates() {
    let mut e = chain_engine();
    e.set_filters(muted(&["svc_orders"]));
    let t = e.timeline_tracks(BASE, BASE + 12 * MS, 12, 100);
    // Only the gateway root request is counted; orders still has its own row and clips.
    assert_eq!(t.requests.iter().sum::<u32>(), 1);
    assert_eq!(t.latency_samples.iter().sum::<u32>(), 1);
    let (oi, orders) = track(&t, "orders");
    assert_eq!(orders.counts.iter().sum::<u32>(), 2);
    assert!(t.clips.iter().any(|c| c.track as usize == oi));
}

#[test]
fn spans_that_fit_their_caller_are_not_shifted() {
    let mut e = chain_engine();
    let t = e.timeline_tracks(BASE, BASE + 12 * MS, 12, 100);
    let (orders, _) = track(&t, "orders");
    assert!(t.clips.iter().any(|c| c.track == orders as u32 && c.start_ms == 1.0));
    e.seek(BASE + 5 * MS, 1);
    let vs = e.visible_state();
    assert!(hops(&vs).iter().all(|h| h[5] as u32 & HOP_CLAMPED == 0));
}

#[test]
fn hops_follow_filters_and_trace_selection() {
    let mut e = chain_engine();
    e.seek(BASE + 5 * MS, 1);
    e.set_filters(Filters { selected_trace_id: Some("t1".into()), ..Filters::default() });
    let vs = e.visible_state();
    assert!(hops(&vs).iter().all(|h| h[5] as u32 & HOP_HIGHLIGHT != 0));
    // Hiding the inferred database edge hides its hops too.
    e.set_filters(Filters { hide_inferred: true, ..Filters::default() });
    let vs = e.visible_state();
    assert_eq!(hops(&vs).len(), 1);
}

#[test]
fn hops_update_when_new_spans_arrive_and_are_evicted() {
    let mut e = chain_engine();
    e.seek(BASE + 5 * MS, 1);
    assert_eq!(hops(&e.visible_state()).len(), 2);
    e.apply_batch(Batch {
        items: vec![
            TimelineItem::Span(span("t2", "g2", None, "gateway", 3, 4 * MS, 6 * MS, false)),
            TimelineItem::Span(span("t2", "o1", Some("g2"), "orders", 2, 4 * MS + US, 6 * MS - US, false)),
        ],
    })
    .unwrap();
    e.seek(BASE + 5 * MS, 2);
    assert_eq!(hops(&e.visible_state()).len(), 3);
    e.evict_before(BASE + 20 * MS);
    e.seek(BASE + 5 * MS, 3);
    assert!(e.visible_state().hop_data.is_empty());
}

fn node_index(e: &Engine, id: &str) -> f32 {
    e.topology().nodes.iter().find(|n| n.id == id).unwrap().index as f32
}

/// chain_engine plus a second trace `t2` (gateway -> orders at 4..6 ms) and an unrelated log.
fn two_trace_engine() -> Engine {
    let mut e = chain_engine();
    e.apply_batch(Batch {
        items: vec![
            TimelineItem::Span(span("t2", "g2", None, "gateway", 3, 4 * MS, 6 * MS, false)),
            TimelineItem::Span(span("t2", "o1", Some("g2"), "orders", 2, 4 * MS + US, 6 * MS - US, true)),
            TimelineItem::Log(LogItem { trace_id: Some("t1".into()), ..log("l1", "orders", 3 * MS, 17) }),
            TimelineItem::Log(LogItem { trace_id: Some("t2".into()), ..log("l2", "orders", 5 * MS, 17) }),
        ],
    })
    .unwrap();
    e
}

#[test]
fn hops_carry_the_service_where_their_trace_started() {
    let mut e = chain_engine();
    e.seek(BASE + 5 * MS, 1);
    let gateway = node_index(&e, "svc_gateway");
    let vs = e.visible_state();
    assert_eq!(hops(&vs).len(), 2);
    // Both the gateway -> orders call and the orders -> database call belong to a request that
    // entered the system at the gateway.
    assert!(hops(&vs).iter().all(|h| h[6] == gateway), "{:?}", hops(&vs));

    // Root not loaded: the topmost loaded ancestor's service is the origin.
    let mut e = engine_with(vec![
        TimelineItem::Span(span("t9", "o1", Some("missing"), "orders", 2, 0, 10 * MS, false)),
        TimelineItem::Span(span("t9", "o2", Some("o1"), "orders", 3, MS, 9 * MS, false)),
        TimelineItem::Span(span("t9", "p1", Some("o2"), "payments", 2, 2 * MS, 8 * MS, false)),
    ]);
    e.seek(BASE + 5 * MS, 1);
    let orders = node_index(&e, "svc_orders");
    let vs = e.visible_state();
    assert_eq!(hops(&vs).len(), 1);
    assert_eq!(hops(&vs)[0][6], orders);
}

#[test]
fn isolated_trace_shows_only_its_hops_and_dims_everything_off_its_path() {
    let mut e = two_trace_engine();
    e.seek(BASE + 5 * MS, 1);
    assert_eq!(hops(&e.visible_state()).len(), 3);

    // Selecting without isolation only highlights.
    e.set_filters(Filters { selected_trace_id: Some("t2".into()), ..Filters::default() });
    let vs = e.visible_state();
    assert_eq!(hops(&vs).len(), 3);
    assert!(vs.node_data.chunks(NODE_STRIDE).all(|n| n[6] as u32 & FLAG_DIMMED == 0));

    e.set_filters(Filters { selected_trace_id: Some("t2".into()), trace_only: true, ..Filters::default() });
    let vs = e.visible_state();
    let call = edge_between(&e, "svc_gateway", "svc_orders");
    let db = edge_between(&e, "svc_orders", "res_db");
    let hs = hops(&vs);
    assert_eq!(hs.len(), 1, "only the t2 call: {hs:?}");
    assert_eq!(hs[0][0] as u32, call);
    assert_eq!(hs[0][5] as u32 & (HOP_HIGHLIGHT | HOP_DIMMED | HOP_ERROR), HOP_HIGHLIGHT | HOP_ERROR);
    assert_eq!(vs.summary.hops_in_window, 1);
    assert_eq!(node_flags(&e, &vs, "svc_gateway") & FLAG_DIMMED, 0);
    assert_eq!(node_flags(&e, &vs, "svc_orders") & FLAG_DIMMED, 0);
    assert_eq!(node_flags(&e, &vs, "res_db") & FLAG_DIMMED, FLAG_DIMMED, "t2 never touches the database");
    assert_eq!(edge_flags(&vs, call) & FLAG_DIMMED, 0);
    assert_eq!(edge_flags(&vs, db) & FLAG_DIMMED, FLAG_DIMMED);

    // trace_only without a selected trace changes nothing.
    e.set_filters(Filters { trace_only: true, ..Filters::default() });
    let vs = e.visible_state();
    assert_eq!(hops(&vs).len(), 3);
    assert!(vs.node_data.chunks(NODE_STRIDE).all(|n| n[6] as u32 & FLAG_DIMMED == 0));
    // An unknown trace isolates nothing to show.
    e.set_filters(Filters { selected_trace_id: Some("nope".into()), trace_only: true, ..Filters::default() });
    let vs = e.visible_state();
    assert!(vs.hop_data.is_empty());
    assert!(vs.node_data.chunks(NODE_STRIDE).all(|n| n[6] as u32 & FLAG_DIMMED != 0));
}

#[test]
fn isolated_trace_limits_the_timeline_tracks_to_that_trace() {
    let mut e = two_trace_engine();
    e.apply_batch(Batch {
        items: vec![TimelineItem::Span(span("t3", "p1", None, "payments", 2, 2 * MS, 3 * MS, false))],
    })
    .unwrap();
    let all = e.timeline_tracks(BASE, BASE + 12 * MS, 12, 100);
    assert_eq!(all.services.len(), 3);

    e.set_filters(Filters { selected_trace_id: Some("t2".into()), trace_only: true, ..Filters::default() });
    let t = e.timeline_tracks(BASE, BASE + 12 * MS, 12, 100);
    let names: Vec<&str> = t.services.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"gateway") && names.contains(&"orders"));
    // t2: the gateway root and the orders server span are requests; orders failed; its error log.
    assert_eq!(t.requests.iter().sum::<u32>(), 2);
    assert_eq!(t.errors.iter().sum::<u32>(), 2, "failed span + t2's error log, not t1's");
    assert!(t.clips.iter().all(|c| c.trace_id == "t2"), "{:?}", t.clips);
    assert_eq!(t.clips.len(), 2);
}

/// Sub-interval of `[lo, hi]` between the fractions `a` and `b` (per mille, in either order).
fn within(lo: i64, hi: i64, a: i64, b: i64) -> (i64, i64) {
    let (a, b) = (a.min(b), a.max(b));
    (lo + (hi - lo) * a / 1000, lo + (hi - lo) * b / 1000)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(160))]

    /// However far the services' clocks drift apart, the animation stays causal: a request
    /// reaches a service before that service calls on, the callee's calls return before it
    /// answers, and calls a service makes one after another stay in that order.
    #[test]
    fn nested_hops_stay_in_causal_order_under_clock_skew(
        skew in prop::collection::vec(-5_000i64..5_000, 3),
        f in prop::collection::vec(0i64..=1000, 10),
    ) {
        // True times in µs; gateway -> orders -> payments, then orders -> database.
        let root = (0, 20_000);
        let gw_call = within(root.0, root.1, f[0], f[1]);
        let orders = within(gw_call.0, gw_call.1, f[2], f[3]);
        let mid = (orders.0 + orders.1) / 2;
        let pay_call = within(orders.0, mid, f[4], f[5]);
        let payments = within(pay_call.0, pay_call.1, f[6], f[7]);
        let db_call = within(mid, orders.1, f[8], f[9]);
        let rec = |(s, e): (i64, i64), k: i64| ((s + k) * US, (e + k) * US);
        let sp = |id: &str, parent: Option<&str>, svc: &str, kind: u8, t: (i64, i64)| span("t", id, parent, svc, kind, t.0, t.1, false);
        let (g, o, p) = (skew[0], skew[1], skew[2]);
        let items = vec![
            sp("g0", None, "gateway", 2, rec(root, g)),
            sp("g1", Some("g0"), "gateway", 3, rec(gw_call, g)),
            sp("o0", Some("g1"), "orders", 2, rec(orders, o)),
            sp("o1", Some("o0"), "orders", 3, rec(pay_call, o)),
            sp("p0", Some("o1"), "payments", 2, rec(payments, p)),
            SpanItem { target_entity_id: Some("res_db".into()), ..sp("o2", Some("o0"), "orders", 3, rec(db_call, o)) },
        ];
        let mut e = Engine::new(EngineConfig::default());
        e.load_topology(TopologySnapshot {
            nodes: vec![TopologyNode { id: "res_db".into(), name: "postgres".into(), kind: EntityKindDto::Database, provenance: ProvenanceDto::Inferred }],
            edges: vec![],
        });
        e.apply_batch(Batch { items: items.into_iter().map(TimelineItem::Span).collect() }).unwrap();
        e.set_hop_window(60 * SEC, 60 * SEC);
        e.seek(BASE, 1);
        let vs = e.visible_state();
        let hs = hops(&vs);
        prop_assert_eq!(hs.len(), 3);
        let by = |from: &str, to: &str| -> Vec<f32> {
            let edge = edge_between(&e, from, to) as f32;
            hs.iter().find(|h| h[0] == edge).map(|h| h.to_vec()).unwrap_or_default()
        };
        let (gw_o, o_p, o_db) = (by("svc_gateway", "svc_orders"), by("svc_orders", "svc_payments"), by("svc_orders", "res_db"));
        prop_assert!(!gw_o.is_empty() && !o_p.is_empty() && !o_db.is_empty());
        let eps = 1e-3;
        for h in [&gw_o, &o_p, &o_db] {
            prop_assert!(h[1] <= h[2] && h[2] <= h[3] && h[3] <= h[4], "{:?}", h);
            prop_assert_eq!(h[6], node_index(&e, "svc_gateway"));
        }
        for child in [&o_p, &o_db] {
            prop_assert!(child[1] + eps >= gw_o[2], "orders calls on before the request arrived: {:?} {:?}", gw_o, child);
            prop_assert!(child[4] <= gw_o[3] + eps, "orders answers before its call returned: {:?} {:?}", gw_o, child);
        }
        prop_assert!(o_p[4] <= o_db[1] + eps, "sequential calls swapped: {:?} {:?}", o_p, o_db);
    }
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
                SpanItem {
                    source_entity_id: (id % 2 == 0).then(|| "ext_clients".into()),
                    ..span(
                        &format!("t{}", id % 7),
                        &format!("s{id}"),
                        parent.as_deref(),
                        s,
                        kind,
                        start * SEC,
                        (start + dur) * SEC,
                        err,
                    )
                }
            })
            .collect::<Vec<_>>()
    });
    let logs = prop::collection::vec((0u32..40, svc, 0i64..130, prop::sample::select(vec![9u8, 13, 17])), 0..60)
        .prop_map(|v| v.into_iter().map(|(id, s, t, sev)| log(&format!("l{id}"), s, t * SEC, sev)).collect::<Vec<_>>());
    (spans, logs)
}

fn external_client_topology() -> TopologySnapshot {
    TopologySnapshot {
        nodes: vec![ext_node("ext_clients", "External clients", EntityKindDto::ExternalClient)],
        edges: vec![],
    }
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
        a.load_topology(external_client_topology());
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
        b.load_topology(external_client_topology());
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

    /// Incremental merges (a seek between every batch) give the same state as one full build,
    /// including after eviction.
    #[test]
    fn incremental_matches_full_build(
        (spans, logs) in arb_items(),
        chunks in 1usize..8,
        t in 0i64..140,
        evict in prop::option::of(0i64..60),
        small_budget in any::<bool>(),
    ) {
        let (spans, logs) = dedup_last_wins(&spans, &logs);
        let mut items: Vec<TimelineItem> = spans.iter().cloned().map(TimelineItem::Span)
            .chain(logs.iter().cloned().map(TimelineItem::Log)).collect();
        items.sort_by_key(|i| match i {
            TimelineItem::Span(s) => s.end_ns,
            TimelineItem::Log(l) => l.time_ns,
            TimelineItem::Event(e) => e.time_ns,
        });
        let cfg = EngineConfig {
            checkpoint_budget_bytes: if small_budget { 1 } else { 16 << 20 },
            ..EngineConfig::default()
        };

        let mut inc = Engine::new(cfg.clone());
        inc.load_topology(external_client_topology());
        let size = items.len().div_ceil(chunks).max(1);
        for (g, chunk) in items.chunks(size).enumerate() {
            inc.apply_batch(Batch { items: chunk.to_vec() }).unwrap();
            inc.seek(BASE + t * SEC, g as u64 + 1);
        }
        let mut full = Engine::new(cfg);
        full.load_topology(external_client_topology());
        full.apply_batch(Batch { items }).unwrap();
        if let Some(ev) = evict {
            inc.evict_before(BASE + ev * SEC);
            full.evict_before(BASE + ev * SEC);
        }

        let tt = BASE + t * SEC;
        inc.seek(tt, 100);
        full.seek(tt, 100);
        prop_assert_eq!(inc.prev_event_time(tt), full.prev_event_time(tt));
        prop_assert_eq!(inc.next_event_time(tt), full.next_event_time(tt));
        for n in full.topology().nodes {
            let a = inc.node_metrics(&n.id, None);
            let b = full.node_metrics(&n.id, None);
            prop_assert_eq!(a, b, "node {}", n.id);
        }
    }
}

#[test]
fn live_batches_merge_instead_of_rebuilding() {
    let mut e = Engine::new(EngineConfig::default());
    let first = vec![
        TimelineItem::Span(span("t1", "a", None, "gateway", 2, 0, SEC, false)),
        TimelineItem::Span(span("t1", "b", Some("a"), "orders", 2, 0, SEC, false)),
    ];
    e.apply_batch(Batch { items: first }).unwrap();
    e.seek(BASE + SEC, 1);
    assert_eq!(e.diagnostics().timeline_rebuilds, 1);

    for i in 1..=5i64 {
        let trace = format!("t{}", i + 1);
        let items = vec![
            TimelineItem::Span(span(&trace, "a", None, "gateway", 2, i * SEC, (i + 1) * SEC, false)),
            TimelineItem::Span(span(&trace, "b", Some("a"), "orders", 2, i * SEC, (i + 1) * SEC, i == 3)),
            TimelineItem::Log(log(&format!("l{i}"), "orders", i * SEC, 9)),
        ];
        e.apply_batch(Batch { items }).unwrap();
        e.seek(BASE + (i + 1) * SEC, i as u64 + 1);
    }
    let d = e.diagnostics();
    assert_eq!(d.timeline_rebuilds, 1, "known nodes and edges must not trigger rebuilds");
    assert_eq!(d.timeline_merges, 5);
    let m = e.node_metrics("svc_orders", Some(BASE + 6 * SEC)).unwrap();
    assert_eq!(m.window_errors, 1);

    // A child whose parent arrives later is structural: the late parent forces a rebuild.
    e.apply_batch(Batch { items: vec![TimelineItem::Span(span("t9", "c", Some("late"), "orders", 2, 0, SEC, false))] })
        .unwrap();
    e.seek(BASE + SEC, 20);
    e.apply_batch(Batch { items: vec![TimelineItem::Span(span("t9", "late", None, "gateway", 3, 0, SEC, false))] })
        .unwrap();
    e.seek(BASE + SEC, 21);
    assert_eq!(e.diagnostics().timeline_rebuilds, 2);
}

// ------------------------------------------------------------------ timeline tracks

fn track<'a>(t: &'a TimelineTracks, name: &str) -> (usize, &'a ServiceTrack) {
    t.services.iter().enumerate().find(|(_, s)| s.name == name).unwrap()
}

#[test]
fn tracks_count_requests_on_the_callee_and_p95_by_end_bucket() {
    let e = chain_engine();
    let t = e.timeline_tracks(BASE, BASE + 12 * MS, 12, 100);
    // g1 (root) and o1 (server); client spans are not counted again.
    assert_eq!(t.requests[0], 1);
    assert_eq!(t.requests[1], 1);
    assert_eq!(t.requests.iter().sum::<u32>(), 2);
    // d1 failed but is a client span; no request span failed and there are no error logs.
    assert_eq!(t.errors.iter().sum::<u32>(), 0);
    assert_eq!(t.latency_p95_ms[11], Some(11.0));
    assert_eq!(t.latency_p95_ms[9], Some(8.0));
    assert_eq!(t.latency_p95_ms[5], None);
    assert_eq!(t.latency_samples[11], 1);

    let (_, gw) = track(&t, "gateway");
    assert_eq!(gw.counts[0], 2);
    let (oi, orders) = track(&t, "orders");
    assert_eq!((orders.counts[1], orders.counts[2]), (1, 1));
    assert!(orders.clipped);
    let clips: Vec<&SpanClip> = t.clips.iter().filter(|c| c.track as usize == oi).collect();
    assert_eq!(clips.len(), 2);
    assert_eq!((clips[0].start_ms, clips[0].end_ms, clips[0].depth, clips[0].error), (1.0, 9.0, 0, false));
    assert!((clips[1].start_ms - 2.005).abs() < 1e-9, "{}", clips[1].start_ms);
    assert_eq!((clips[1].end_ms, clips[1].depth, clips[1].error), (7.0, 1, true));
}

#[test]
fn tracks_p95_is_nearest_rank() {
    let items =
        (1..=20).map(|i| TimelineItem::Span(span(&format!("t{i}"), "r", None, "api", 2, 0, i * MS, false))).collect();
    let e = engine_with(items);
    let t = e.timeline_tracks(BASE, BASE + 100 * MS, 1, 0);
    assert_eq!(t.requests, vec![20]);
    assert_eq!(t.latency_samples, vec![20]);
    assert_eq!(t.latency_p95_ms, vec![Some(19.0)]);
}

#[test]
fn tracks_leave_out_clips_over_the_cap_but_report_the_count() {
    let e = chain_engine();
    let t = e.timeline_tracks(BASE, BASE + 12 * MS, 12, 1);
    let (_, orders) = track(&t, "orders");
    assert_eq!(orders.spans_in_view, 2);
    assert!(!orders.clipped);
    assert!(t.clips.is_empty());
}

#[test]
fn tracks_honour_service_filters() {
    let mut e = chain_engine();
    e.set_filters(Filters { search: "orders".into(), ..Filters::default() });
    let t = e.timeline_tracks(BASE, BASE + 12 * MS, 12, 100);
    assert_eq!(t.services.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["orders"]);
    assert_eq!(t.requests.iter().sum::<u32>(), 1);
}

#[test]
fn tracks_count_error_logs_events_and_mark_rare_event_types() {
    let mut items = demo_items();
    items.push(TimelineItem::Event(EventItem {
        id: "e2".into(),
        event_type: "deployment.started".into(),
        service_id: Some("svc_orders".into()),
        service_name: Some("orders".into()),
        time_ns: BASE + 5 * SEC,
        trace_id: None,
    }));
    let e = engine_with(items);
    let t = e.timeline_tracks(BASE, BASE + 10 * SEC, 10, 0);
    assert_eq!(t.errors[2], 1, "failed orders server span");
    assert_eq!(t.errors[3], 1, "error log");
    assert_eq!(t.events[0], 1);
    assert_eq!(t.events[5], 1);
    assert_eq!(t.markers_total, 1);
    assert_eq!(t.markers[0].event_type, "deployment.started");
    assert_eq!(t.markers[0].time_ms, 5000.0);
}

#[test]
fn tracks_for_an_empty_window_are_empty() {
    let e = chain_engine();
    let t = e.timeline_tracks(BASE, BASE, 10, 10);
    assert!(t.requests.is_empty() && t.services.is_empty());
    let t = e.timeline_tracks(BASE + SEC, BASE + 2 * SEC, 4, 10);
    assert_eq!(t.requests, vec![0; 4]);
    assert!(t.clips.is_empty());
}

// ---------------------------------------------------------------- infrastructure layer

fn infra_json() -> String {
    serde_json::json!({
        "at_ns": (BASE + 5 * SEC).to_string(),
        "window_ns": (120 * SEC).to_string(),
        "hosts": [
            {"id": "vps-app", "name": "vps-app", "has_metrics": true, "cpu_utilization": 0.92,
             "memory_used_bytes": 912_261_120.0, "memory_total_bytes": 1_073_741_824.0, "containers": []},
            {"id": "vps-worker", "name": "vps-worker", "has_metrics": true, "cpu_utilization": 0.2, "containers": []}
        ],
        "instances": [
            {"service_id": "svc_gateway", "service_name": "gateway", "instance_id": "g1", "host_id": "vps-app", "match_reason": "container_id", "requests": 3},
            {"service_id": "svc_orders", "service_name": "orders", "instance_id": "o1", "host_id": "vps-worker", "match_reason": "host_name", "requests": 3},
            {"service_id": "svc_orders", "service_name": "orders", "instance_id": "o2", "host_id": "vps-app", "match_reason": "host_name", "requests": 3},
            {"service_id": "svc_orders", "service_name": "orders", "instance_id": "o3", "host_id": "vps-worker", "match_reason": "host_name", "requests": 3}
        ],
        "hints": [],
        "notes": []
    })
    .to_string()
}

fn infra_on(e: &mut Engine, on: bool) {
    e.set_filters(Filters { infrastructure: on, ..Filters::default() });
}

#[test]
fn infrastructure_layer_moves_cards_into_machine_frames_and_back() {
    let mut e = engine_with(demo_items());
    e.seek(BASE + 5 * SEC, 1);
    let layered = e.topology();
    assert!(layered.infra.is_none());
    e.set_infrastructure_json(&infra_json()).unwrap();
    assert_eq!(e.topology().revision, layered.revision, "a snapshot alone does not change the map");

    infra_on(&mut e, true);
    let topo = e.topology();
    assert!(topo.revision > layered.revision, "the renderer must redraw");
    let view = topo.infra.as_ref().unwrap();
    let frame = |id: &str| view.frames.iter().find(|f| f.host_id == id).unwrap();
    assert_eq!(frame("vps-app").label, "vps-app · CPU 92% · RAM 870/1024 MB");
    let gateway = topo.nodes.iter().find(|n| n.id == "svc_gateway").unwrap();
    let app = frame("vps-app");
    assert!(gateway.x > app.x && gateway.x < app.x + app.width && gateway.y > app.y && gateway.y < app.y + app.height);
    assert_eq!(view.badges, vec![InfraBadge { node: 1, count: 3, hosts: 2 }]);
    assert_eq!(view.chips.len(), 3);
    assert!(topo.edges.iter().all(|edge| edge.points.is_empty()), "no layered corridors inside frames");

    // Frames drive the frame geometry too, not only the topology view.
    let vs = e.visible_state();
    let i = vs.node_indices.iter().position(|&n| n == 0).unwrap();
    assert_eq!((vs.node_data[i * NODE_STRIDE], vs.node_data[i * NODE_STRIDE + 1]), (gateway.x, gateway.y));
    assert_eq!(e.pick(gateway.x, gateway.y).as_deref(), Some("svc_gateway"));
    let chip = &view.chips[0];
    assert_eq!(e.pick(chip.x + 60.0, chip.y).as_deref(), Some("svc_orders"), "chips pick their service");

    infra_on(&mut e, false);
    let back = e.topology();
    assert!(back.infra.is_none());
    let g = back.nodes.iter().find(|n| n.id == "svc_gateway").unwrap();
    let g0 = layered.nodes.iter().find(|n| n.id == "svc_gateway").unwrap();
    assert_eq!((g.x, g.y), (g0.x, g0.y), "switching off restores the layered layout");
}

#[test]
fn infrastructure_layer_without_a_snapshot_keeps_the_layered_layout() {
    let mut e = engine_with(demo_items());
    let before = e.topology();
    infra_on(&mut e, true);
    let topo = e.topology();
    assert!(topo.infra.is_none());
    assert_eq!(
        topo.nodes.iter().map(|n| (n.x, n.y)).collect::<Vec<_>>(),
        before.nodes.iter().map(|n| (n.x, n.y)).collect::<Vec<_>>()
    );
}

#[test]
fn identical_snapshots_do_not_bump_the_revision_and_bad_json_is_rejected() {
    let mut e = engine_with(demo_items());
    infra_on(&mut e, true);
    e.set_infrastructure_json(&infra_json()).unwrap();
    let r = e.topology().revision;
    e.set_infrastructure_json(&infra_json()).unwrap();
    assert_eq!(e.topology().revision, r);
    assert!(matches!(e.set_infrastructure_json("{\"hosts\": 5}"), Err(EngineError::InvalidInput(_))));
    assert_eq!(e.topology().revision, r, "a rejected snapshot changes nothing");
}

#[test]
fn nodes_discovered_later_are_placed_after_relayout() {
    let mut e = engine_with(demo_items());
    infra_on(&mut e, true);
    e.set_infrastructure_json(&infra_json()).unwrap();
    e.apply_batch(Batch { items: vec![TimelineItem::Span(span("t9", "x1", None, "payments", 2, 0, SEC, false))] })
        .unwrap();
    let topo = e.topology();
    let view = topo.infra.as_ref().unwrap();
    let right = view.frames.iter().map(|f| f.x + f.width).fold(0.0f32, f32::max);
    let payments = topo.nodes.iter().find(|n| n.id == "svc_payments").unwrap();
    assert!(payments.x > right, "a service on no known machine sits right of the frames");
}
