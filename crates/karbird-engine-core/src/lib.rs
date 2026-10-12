//! karbird viewer engine core.
//!
//! Holds a bounded window of recorded timeline items, reconstructs the
//! displayed system state at any selected time, filters/culls the visible
//! subgraph and produces compact render buffers. Pure Rust: no I/O, no clock,
//! no threads — the same code runs natively (tests, benchmarks) and in WASM.

mod hops;
mod infra;
mod layout;
mod reducer;
pub mod reference;
mod server_layout;
mod skew;
mod tracks;
pub mod types;

use std::collections::{HashMap, HashSet};

pub use hops::{HOP_CLAMPED, HOP_DIMMED, HOP_ERROR, HOP_HIGHLIGHT, HOP_LEAF, HOP_STRIDE};
use hops::{Hop, HopIndex};
use karbird_domain::{EntityKind, HealthStatus, Provenance, RelationKind};
use layout::Layout;
use reducer::*;
pub use server_layout::{SERVER_LAYOUT_VERSION, SavedTopologyGeometry, build_server_geometry};
use skew::SkewCache;
pub use tracks::{MAX_CLIP_CAP, MAX_TRACK_BUCKETS};
pub use types::*;

/// Floats per node in [`VisibleState::node_data`]:
/// `x, y, radius, status, active, window_requests, flags`.
pub const NODE_STRIDE: usize = 7;
/// Floats per edge in [`VisibleState::edge_data`]:
/// `x1, y1, x2, y2, active, window_calls, error_ratio, flags`.
pub const EDGE_STRIDE: usize = 8;

pub const FLAG_HIGHLIGHT: u32 = 1;
pub const FLAG_INFERRED: u32 = 2;
pub const FLAG_DECLARED: u32 = 4;
pub const FLAG_RESOURCE: u32 = 8;
pub const FLAG_PROBLEM: u32 = 16;
/// Node or edge of a muted service (see `Filters::muted_services`): drawn dimmed.
pub const FLAG_DIMMED: u32 = 32;
/// Node outside the project's architecture (external dependency or external client), or an
/// edge touching one.
pub const FLAG_EXTERNAL: u32 = 64;
/// Slices of the rolling window in [`EdgeMetrics::calls_series`].
pub const EDGE_SERIES_BUCKETS: usize = 15;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SeekOutcome {
    Applied,
    /// A newer generation was already applied; this result must be discarded.
    Stale,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VisibleState {
    pub node_data: Vec<f32>,
    pub node_indices: Vec<u32>,
    pub edge_data: Vec<f32>,
    pub edge_indices: Vec<u32>,
    /// Node indices whose health status changed since the previous frame.
    pub changed_nodes: Vec<u32>,
    /// Individual request hops on visible edges, [`HOP_STRIDE`] floats each.
    pub hop_data: Vec<f32>,
    pub summary: FrameSummary,
}

#[derive(Debug, Clone)]
struct NodeRec {
    id: String,
    name: String,
    lowercase_name: String,
    kind: EntityKind,
    provenance: Provenance,
}

#[derive(Debug, Clone)]
struct EdgeRec {
    source: u32,
    target: u32,
    relation: RelationKind,
    provenance: Provenance,
    reasons: Vec<String>,
}

#[derive(Debug)]
pub struct Engine {
    config: EngineConfig,
    nodes: Vec<NodeRec>,
    node_index: HashMap<String, u32>,
    edges: Vec<EdgeRec>,
    edge_index: HashMap<(u32, u32, RelationKind), u32>,
    layout: Layout,
    spans: HashMap<(String, String), SpanItem>,
    logs: HashMap<String, LogItem>,
    events: HashMap<String, EventItem>,
    timeline: Option<Timeline>,
    unresolved_parents: u32,
    viewport: Option<Viewport>,
    filters: Filters,
    lowercase_search: String,
    current_time: Option<i64>,
    last_generation: u64,
    last_status: Vec<u8>,
    topology_revision: u64,
    duplicates_ignored: u64,
    evicted_items: u64,
    evicted_before: Option<i64>,
    stale_seeks: u64,
    /// Items added since the timeline was last built or merged.
    pending: Vec<ItemKey>,
    /// `(trace_id, parent_span_id)` of spans whose parent was not loaded at build time.
    unresolved_keys: HashSet<(String, String)>,
    timeline_rebuilds: u64,
    timeline_merges: u64,
    /// Built lazily from `spans`; dropped whenever spans or edges change.
    hops: Option<HopIndex>,
    hop_window: (i64, i64),
    /// Nodes or edges were added since the layout was computed.
    layout_dirty: bool,
    infra: Option<InfrastructureSnapshot>,
    /// Machine-frame layout for `infra`; recomputed with the layered layout.
    infra_layout: Option<infra::InfraLayout>,
    server_geometry: Option<SavedTopologyGeometry>,
}

#[derive(Debug, Clone)]
enum ItemKey {
    Span((String, String)),
    Log(String),
    Event(String),
}

enum EdgeLookup {
    NoEdge,
    Existing(u32),
    /// The edge does not exist yet: topology changes, so the timeline must be rebuilt.
    Missing,
    Unresolved((String, String)),
}

fn push_span_deltas(out: &mut Vec<Delta>, s: &SpanItem, node: u32, edge: Option<u32>) {
    let entry = s.kind == 2 || s.kind == 5 || s.parent_span_id.is_none();
    let tb = stable_hash(&[&s.trace_id, &s.span_id]);
    out.push(Delta {
        time: s.start_ns,
        tiebreak: tb,
        evict_at: s.end_ns,
        kind: DeltaKind::SpanStart { node, edge, entry },
    });
    out.push(Delta {
        time: s.end_ns,
        tiebreak: tb ^ 1,
        evict_at: s.end_ns,
        kind: DeltaKind::SpanEnd {
            node,
            edge,
            entry,
            error: s.is_error,
            duration_ns: s.end_ns.saturating_sub(s.start_ns),
        },
    });
}

fn log_delta(l: &LogItem, node: u32) -> Delta {
    Delta {
        time: l.time_ns,
        tiebreak: stable_hash(&[&l.id]),
        evict_at: l.time_ns,
        kind: DeltaKind::Log { node, error: l.severity >= 17 },
    }
}

fn event_delta(e: &EventItem, node: Option<u32>) -> Delta {
    Delta { time: e.time_ns, tiebreak: stable_hash(&[&e.id]), evict_at: e.time_ns, kind: DeltaKind::Event { node } }
}

fn stable_hash(parts: &[&str]) -> u64 {
    // FNV-1a: deterministic across native and wasm (unlike RandomState).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            hop_window: (config.hop_before_ns.max(0), config.hop_after_ns.max(0)),
            config,
            nodes: Vec::new(),
            node_index: HashMap::new(),
            edges: Vec::new(),
            edge_index: HashMap::new(),
            layout: Layout::default(),
            spans: HashMap::new(),
            logs: HashMap::new(),
            events: HashMap::new(),
            timeline: None,
            unresolved_parents: 0,
            viewport: None,
            filters: Filters::default(),
            lowercase_search: String::new(),
            current_time: None,
            last_generation: 0,
            last_status: Vec::new(),
            topology_revision: 0,
            duplicates_ignored: 0,
            evicted_items: 0,
            evicted_before: None,
            stale_seeks: 0,
            pending: Vec::new(),
            unresolved_keys: HashSet::new(),
            timeline_rebuilds: 0,
            timeline_merges: 0,
            hops: None,
            layout_dirty: false,
            infra: None,
            infra_layout: None,
            server_geometry: None,
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    // ---------------------------------------------------------------- input

    /// Replace topology (nodes/edges known from the server). Recorded items are kept.
    pub fn load_topology(&mut self, snapshot: TopologySnapshot) {
        self.replace_topology(snapshot, None);
    }

    fn replace_topology(&mut self, snapshot: TopologySnapshot, geometry: Option<SavedTopologyGeometry>) {
        self.server_geometry = geometry;
        let old_nodes = std::mem::take(&mut self.nodes);
        self.node_index.clear();
        self.edges.clear();
        self.edge_index.clear();
        for n in snapshot.nodes {
            self.ensure_node(&n.id, &n.name, n.kind.into(), n.provenance.into());
        }
        // Keep nodes discovered from items so their items remain attributable.
        for n in old_nodes {
            if self.server_geometry.is_none() && !self.node_index.contains_key(&n.id) {
                self.ensure_node(&n.id, &n.name, n.kind, n.provenance);
            }
        }
        for e in snapshot.edges {
            let (Some(&s), Some(&t)) = (self.node_index.get(&e.source), self.node_index.get(&e.target)) else {
                continue;
            };
            let idx = self.ensure_edge(s, t, e.relation.into(), e.provenance.into());
            if let Some(rec) = self.edges.get_mut(idx as usize) {
                rec.reasons = e.reasons;
            }
        }
        if self.server_geometry.is_some() {
            self.layout_dirty = false;
            self.refresh_infra();
            self.topology_revision += 1;
        } else {
            self.relayout();
        }
        self.timeline = None;
        self.hops = None;
    }

    pub fn load_topology_json(&mut self, json: &str) -> Result<(), EngineError> {
        #[derive(serde::Deserialize)]
        struct Response {
            #[serde(flatten)]
            snapshot: TopologySnapshot,
            geometry: Option<SavedTopologyGeometry>,
            infrastructure_snapshot: Option<InfrastructureSnapshot>,
        }
        let response: Response = serde_json::from_str(json).map_err(|e| EngineError::InvalidInput(e.to_string()))?;
        if self.config.server_owned_topology && response.geometry.is_none() {
            return Err(EngineError::InvalidInput("server topology geometry is required".into()));
        }
        if let Some(geometry) = &response.geometry {
            geometry.validate(&response.snapshot)?;
            if self.server_geometry.as_ref() == Some(geometry) {
                if let Some(snapshot) = response.infrastructure_snapshot {
                    self.set_infrastructure(snapshot);
                }
                return Ok(());
            }
        }
        self.replace_topology(response.snapshot, response.geometry);
        if let Some(snapshot) = response.infrastructure_snapshot {
            self.set_infrastructure(snapshot);
        }
        Ok(())
    }

    /// Apply a batch of items. Idempotent per item ID; order-independent. A batch with an
    /// invalid item is rejected as a whole.
    pub fn apply_batch(&mut self, batch: Batch) -> Result<(), EngineError> {
        let mut span_count = 0;
        let mut log_count = 0;
        let mut event_count = 0;
        for item in &batch.items {
            match item {
                TimelineItem::Span(s) => {
                    span_count += 1;
                    if s.end_ns < s.start_ns {
                        return Err(EngineError::InvalidInput(format!("span {} ends before it starts", s.span_id)));
                    }
                }
                TimelineItem::Log(_) => log_count += 1,
                TimelineItem::Event(_) => event_count += 1,
            }
        }
        if self.spans.is_empty() {
            self.spans.reserve(span_count);
        }
        if self.logs.is_empty() {
            self.logs.reserve(log_count);
        }
        if self.events.is_empty() {
            self.events.reserve(event_count);
        }
        let before_nodes = self.nodes.len();
        for item in batch.items {
            match item {
                TimelineItem::Span(s) => {
                    if self.server_geometry.is_none() && !self.config.server_owned_topology {
                        self.ensure_node(&s.service_id, &s.service_name, EntityKind::Service, Provenance::Observed);
                    }
                    let key = (s.trace_id.clone(), s.span_id.clone());
                    match self.spans.get(&key) {
                        None => {
                            if self.timeline.is_some() {
                                self.pending.push(ItemKey::Span(key.clone()));
                            }
                            self.spans.insert(key, s);
                            self.hops = None;
                        }
                        Some(old) => {
                            self.duplicates_ignored += 1;
                            if *old != s {
                                self.spans.insert(key, s);
                                self.timeline = None;
                                self.hops = None;
                            }
                        }
                    }
                }
                TimelineItem::Log(l) => {
                    if self.server_geometry.is_none() && !self.config.server_owned_topology {
                        self.ensure_node(&l.service_id, &l.service_name, EntityKind::Service, Provenance::Observed);
                    }
                    match self.logs.get(&l.id) {
                        None => {
                            if self.timeline.is_some() {
                                self.pending.push(ItemKey::Log(l.id.clone()));
                            }
                            self.logs.insert(l.id.clone(), l);
                        }
                        Some(old) => {
                            self.duplicates_ignored += 1;
                            if *old != l {
                                self.logs.insert(l.id.clone(), l);
                                self.timeline = None;
                            }
                        }
                    }
                }
                TimelineItem::Event(e) => {
                    if let (Some(id), Some(name)) = (&e.service_id, &e.service_name) {
                        let (id, name) = (id.clone(), name.clone());
                        if self.server_geometry.is_none() && !self.config.server_owned_topology {
                            self.ensure_node(&id, &name, EntityKind::Service, Provenance::Observed);
                        }
                    }
                    match self.events.get(&e.id) {
                        None => {
                            if self.timeline.is_some() {
                                self.pending.push(ItemKey::Event(e.id.clone()));
                            }
                            self.events.insert(e.id.clone(), e);
                        }
                        Some(old) => {
                            self.duplicates_ignored += 1;
                            if *old != e {
                                self.events.insert(e.id.clone(), e);
                                self.timeline = None;
                            }
                        }
                    }
                }
            }
        }
        self.evict_if_needed();
        if self.nodes.len() != before_nodes {
            // New nodes change the state vector layout: rebuild instead of merging.
            self.relayout();
            self.timeline = None;
        }
        Ok(())
    }

    pub fn apply_batch_json(&mut self, json: &str) -> Result<(), EngineError> {
        let batch: Batch = serde_json::from_str(json).map_err(|e| EngineError::InvalidInput(e.to_string()))?;
        self.apply_batch(batch)
    }

    /// Drop all recorded items older than `before_ns` (explicit window shrink).
    pub fn evict_before(&mut self, before_ns: i64) {
        let n0 = self.item_count();
        self.spans.retain(|_, s| s.end_ns >= before_ns);
        self.logs.retain(|_, l| l.time_ns >= before_ns);
        self.events.retain(|_, e| e.time_ns >= before_ns);
        let removed = n0 - self.item_count();
        if removed > 0 {
            self.hops = None;
            self.evicted_items += removed as u64;
            self.evicted_before = Some(self.evicted_before.map_or(before_ns, |b| b.max(before_ns)));
            // Kept spans keep the edge attribution they had when their (now evicted) parent was loaded.
            if let Some(tl) = self.timeline.as_mut() {
                tl.retain_from(before_ns);
            }
        }
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = Some(viewport);
    }

    pub fn set_filters(&mut self, filters: Filters) {
        let toggled = filters.infrastructure != self.filters.infrastructure;
        let problems_toggled = filters.only_problems != self.filters.only_problems;
        self.lowercase_search = filters.search.to_lowercase();
        self.filters = filters;
        if toggled {
            self.refresh_infra();
        }
        if toggled || problems_toggled {
            self.topology_revision += 1;
        }
    }

    /// Machines and instances for the infrastructure layer; replaces the previous snapshot.
    pub fn set_infrastructure(&mut self, snapshot: InfrastructureSnapshot) {
        if self.infra.as_ref() == Some(&snapshot) {
            return;
        }
        self.infra = Some(snapshot);
        self.refresh_infra();
        if self.filters.infrastructure {
            self.topology_revision += 1;
        }
    }

    pub fn set_infrastructure_json(&mut self, json: &str) -> Result<(), EngineError> {
        #[derive(serde::Deserialize)]
        struct Response {
            #[serde(flatten)]
            snapshot: InfrastructureSnapshot,
            topology: Option<serde_json::Value>,
        }
        let response: Response = serde_json::from_str(json).map_err(|e| EngineError::InvalidInput(e.to_string()))?;
        if let Some(topology) = response.topology {
            self.load_topology_json(
                &serde_json::to_string(&topology).map_err(|e| EngineError::InvalidInput(e.to_string()))?,
            )?;
        }
        self.set_infrastructure(response.snapshot);
        Ok(())
    }

    fn refresh_infra(&mut self) {
        if self.config.server_owned_topology && self.server_geometry.is_none() {
            self.infra_layout = None;
            return;
        }
        if let Some(saved) = &self.server_geometry {
            self.infra_layout = if self.filters.infrastructure {
                saved.infrastructure.as_ref().map(|view| infra::InfraLayout {
                    positions: view.nodes.iter().map(|n| (n.index, (n.x, n.y))).collect(),
                    waypoints: view.edges.iter().map(|e| e.points.iter().map(|p| (p[0], p[1])).collect()).collect(),
                    external: saved.external.iter().copied().collect(),
                    view: view.infra.clone().unwrap_or_default(),
                })
            } else {
                None
            };
            if let (Some(layout), Some(snapshot)) = (&mut self.infra_layout, &self.infra) {
                for frame in &mut layout.view.frames {
                    if let Some(host) = snapshot.hosts.iter().find(|h| h.id == frame.host_id) {
                        frame.label = infra::frame_label(host);
                        frame.hot = host.cpu_utilization.is_some_and(|v| v >= infra::HOT_CPU)
                            || snapshot.hints.iter().any(|hint| hint.host_id == host.id);
                        frame.has_metrics = host.has_metrics;
                    } else {
                        let name = frame.label.split(" · ").next().unwrap_or(&frame.host_id).to_owned();
                        frame.label = format!("{name} · no collector metrics");
                        frame.hot = false;
                        frame.has_metrics = false;
                    }
                }
                let node_of = self.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i as u32)).collect();
                layout.view.links =
                    infra::noisy_links(snapshot, &node_of, &layout.positions, &layout.view.chips, &layout.view.frames);
            }
            return;
        }
        self.infra_layout = match (&self.infra, self.filters.infrastructure) {
            (Some(snap), true) => {
                let nodes: Vec<infra::LayoutNode<'_>> = self
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| infra::LayoutNode {
                        index: i as u32,
                        id: &n.id,
                        name: &n.name,
                        service: n.kind == EntityKind::Service,
                    })
                    .collect();
                let edges: Vec<(u32, u32)> = self.edges.iter().map(|e| (e.source, e.target)).collect();
                Some(infra::compute(&nodes, &edges, snap))
            }
            _ => None,
        };
    }

    /// A database or queue the infrastructure layer found outside every machine with the agent.
    fn infra_external(&self, index: usize) -> bool {
        self.infra_layout.as_ref().is_some_and(|l| l.external.contains(&(index as u32)))
    }

    /// Infrastructure decorations follow the same time-dependent problem filter as service cards.
    fn filtered_infra(&self) -> Option<InfraView> {
        let layout = self.infra_layout.as_ref()?;
        let mut view = layout.view.clone();
        if !self.filters.only_problems {
            return Some(view);
        }

        let (now, past) = self.states_at(self.current_time);
        let window = self.window_for(&now, &past, false);
        let problem_nodes: HashSet<u32> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, node)| {
                node.kind == EntityKind::Service
                    && matches!(window[*i].status, HealthStatus::Degraded | HealthStatus::Failing)
            })
            .map(|(i, _)| i as u32)
            .collect();
        let problem_hosts: HashSet<String> = layout
            .view
            .frames
            .iter()
            .filter(|frame| {
                problem_nodes.iter().any(|node| {
                    layout.positions.get(node).is_some_and(|&(x, y)| {
                        x >= frame.x && x <= frame.x + frame.width && y >= frame.y && y <= frame.y + frame.height
                    })
                }) || layout
                    .view
                    .chips
                    .iter()
                    .any(|chip| problem_nodes.contains(&chip.node) && chip.host_id == frame.host_id)
            })
            .map(|frame| frame.host_id.clone())
            .collect();
        view.frames.retain(|frame| problem_hosts.contains(&frame.host_id));
        view.chips.retain(|chip| problem_nodes.contains(&chip.node) && problem_hosts.contains(&chip.host_id));
        view.badges.retain(|badge| problem_nodes.contains(&badge.node));
        view.links.retain(|link| problem_nodes.contains(&link.victim_node) && problem_hosts.contains(&link.host_id));
        Some(view)
    }

    /// Node position in the current layout (machine frames when the infrastructure layer is on).
    fn pos(&self, index: usize, id: &str) -> Option<(f32, f32)> {
        if let Some(l) = &self.infra_layout
            && let Some(&p) = l.positions.get(&(index as u32))
        {
            return Some(p);
        }
        self.server_geometry
            .as_ref()
            .and_then(|g| g.plain.nodes.iter().find(|n| n.id == id).map(|n| (n.x, n.y)))
            .or_else(|| self.layout.position(id))
    }

    /// Which hops frames carry: those overlapping `[time - before, time + after]`, each side
    /// capped at one minute.
    pub fn set_hop_window(&mut self, before_ns: i64, after_ns: i64) {
        const MAX: i64 = 60_000_000_000;
        self.hop_window = (before_ns.clamp(0, MAX), after_ns.clamp(0, MAX));
    }

    /// Move the selected time. Results of older generations are rejected so a
    /// slow seek can never overwrite a newer one.
    pub fn seek(&mut self, time_ns: i64, generation: u64) -> SeekOutcome {
        if generation < self.last_generation {
            self.stale_seeks += 1;
            return SeekOutcome::Stale;
        }
        self.last_generation = generation;
        self.current_time = Some(time_ns);
        self.ensure_timeline();
        SeekOutcome::Applied
    }

    pub fn current_time(&self) -> Option<i64> {
        self.current_time
    }

    pub fn prev_event_time(&mut self, t: i64) -> Option<i64> {
        self.ensure_timeline();
        self.timeline.as_ref().and_then(|tl| tl.prev_time(t))
    }

    pub fn next_event_time(&mut self, t: i64) -> Option<i64> {
        self.ensure_timeline();
        self.timeline.as_ref().and_then(|tl| tl.next_time(t))
    }

    // ---------------------------------------------------------------- output

    pub fn topology(&self) -> TopologyView {
        TopologyView {
            revision: self.topology_revision,
            nodes: self
                .nodes
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let (x, y) = self.pos(i, &n.id).unwrap_or((0.0, 0.0));
                    TopologyViewNode {
                        index: i as u32,
                        id: n.id.clone(),
                        name: n.name.clone(),
                        kind: n.kind.into(),
                        provenance: n.provenance.into(),
                        x,
                        y,
                    }
                })
                .collect(),
            edges: self
                .edges
                .iter()
                .enumerate()
                .map(|(i, e)| TopologyViewEdge {
                    index: i as u32,
                    source: e.source,
                    target: e.target,
                    relation: e.relation.into(),
                    provenance: e.provenance.into(),
                    reasons: e.reasons.clone(),
                    points: match &self.infra_layout {
                        Some(l) => l.waypoints.get(i).map_or(&[][..], Vec::as_slice),
                        None => match &self.server_geometry {
                            Some(g) => return g.plain.edges[i].clone(),
                            None => self.layout.waypoints(i),
                        },
                    }
                    .iter()
                    .map(|&(x, y)| [x, y])
                    .collect(),
                })
                .collect(),
            infra: self.filtered_infra(),
        }
    }

    /// Compute the visible frame at the current time.
    pub fn visible_state(&mut self) -> VisibleState {
        self.ensure_timeline();
        let t = self.current_time;
        let (now, past) = self.states_at(t);
        let window = self.window_for(&now, &past, false);

        let highlight = self.trace_highlight();
        let search = &self.lowercase_search;
        let lod = match self.viewport.map(|v| v.scale) {
            Some(s) if s < 0.45 => 0,
            Some(s) if s > 1.6 => 2,
            _ => 1,
        };

        let isolated = self.filters.isolated_trace().is_some();
        let mut dimmed = self.dimmed_nodes();
        if isolated {
            for (i, d) in dimmed.iter_mut().enumerate() {
                *d |= !highlight.0.contains(&(i as u32));
            }
        }
        let mut dimmed_edges: HashSet<u32> = HashSet::new();
        let mut node_visible = vec![false; self.nodes.len()];
        // Passed every filter (before viewport culling).
        let mut node_passes = vec![false; self.nodes.len()];
        let mut out = VisibleState::default();
        let mut statuses = vec![0u8; self.nodes.len()];
        let (mut total_active, mut win_req, mut win_err) = (0i64, 0i64, 0i64);

        for (i, n) in self.nodes.iter().enumerate() {
            let m = &window[i];
            statuses[i] = m.status.as_u8();
            // An external client's requests are the root server spans its callee already counts.
            if n.kind != EntityKind::ExternalClient {
                total_active += m.active;
                win_req += m.started;
                win_err += m.errors;
            }

            if self.filters.hide_inferred && n.provenance == Provenance::Inferred {
                continue;
            }
            if !search.is_empty() && !n.lowercase_name.contains(search) && !n.id.contains(search) {
                continue;
            }
            let problem = matches!(m.status, HealthStatus::Degraded | HealthStatus::Failing);
            if self.filters.only_problems && !problem {
                continue;
            }
            node_passes[i] = true;
            let (x, y) = self.pos(i, &n.id).unwrap_or((0.0, 0.0));
            let radius = if n.kind == EntityKind::Service { 26.0 } else { 20.0 };
            if !self.in_viewport(x, y, radius) {
                continue;
            }
            node_visible[i] = true;
            let mut flags = 0u32;
            if highlight.0.contains(&(i as u32)) {
                flags |= FLAG_HIGHLIGHT;
            }
            if n.provenance == Provenance::Inferred {
                flags |= FLAG_INFERRED;
            }
            if n.provenance == Provenance::Declared {
                flags |= FLAG_DECLARED;
            }
            if n.kind != EntityKind::Service {
                flags |= FLAG_RESOURCE;
            }
            if n.kind.is_external() || self.infra_external(i) {
                flags |= FLAG_EXTERNAL;
            }
            if problem {
                flags |= FLAG_PROBLEM;
            }
            if dimmed.get(i).copied().unwrap_or(false) {
                flags |= FLAG_DIMMED;
            }
            out.node_indices.push(i as u32);
            out.node_data.extend_from_slice(&[
                x,
                y,
                radius,
                f32::from(m.status.as_u8()),
                m.active as f32,
                m.started.max(m.completed) as f32,
                flags as f32,
            ]);
        }

        for (i, e) in self.edges.iter().enumerate() {
            let (s, tg) = (e.source as usize, e.target as usize);
            let (Some(&sv), Some(&tv)) = (node_visible.get(s), node_visible.get(tg)) else {
                continue;
            };
            let (Some(&sp), Some(&tp)) = (node_passes.get(s), node_passes.get(tg)) else {
                continue;
            };
            let hidden_by_filter = self.filters.hide_inferred && e.provenance == Provenance::Inferred;
            // Keep edges whose both endpoints pass filters; culling allows one end off-screen.
            if hidden_by_filter || !(sv || tv) || !sp || !tp {
                continue;
            }
            let (Some(sn), Some(tn)) = (self.nodes.get(s), self.nodes.get(tg)) else {
                continue;
            };
            let (x1, y1) = self.pos(s, &sn.id).unwrap_or((0.0, 0.0));
            let (x2, y2) = self.pos(tg, &tn.id).unwrap_or((0.0, 0.0));
            let active = now.edge(i, E_ACTIVE);
            let calls = now.edge(i, E_CALLS) - past.edge(i, E_CALLS);
            let errors = now.edge(i, E_ERRORS) - past.edge(i, E_ERRORS);
            let ratio = if calls > 0 { errors as f32 / calls as f32 } else { 0.0 };
            let mut flags = 0u32;
            if highlight.1.contains(&(i as u32)) {
                flags |= FLAG_HIGHLIGHT;
            }
            match e.provenance {
                Provenance::Inferred => flags |= FLAG_INFERRED,
                Provenance::Declared => flags |= FLAG_DECLARED,
                Provenance::Observed => {}
            }
            if ratio >= self.config.degraded_error_ratio && calls > 0 {
                flags |= FLAG_PROBLEM;
            }
            if sn.kind.is_external() || tn.kind.is_external() || self.infra_external(s) || self.infra_external(tg) {
                flags |= FLAG_EXTERNAL;
            }
            let off_trace = isolated && !highlight.1.contains(&(i as u32));
            if off_trace || dimmed.get(s).copied().unwrap_or(false) || dimmed.get(tg).copied().unwrap_or(false) {
                flags |= FLAG_DIMMED;
                dimmed_edges.insert(i as u32);
            }
            out.edge_indices.push(i as u32);
            out.edge_data.extend_from_slice(&[x1, y1, x2, y2, active as f32, calls as f32, ratio, flags as f32]);
        }

        let (hops_in_window, hops_shown) = match t {
            Some(t) => self.collect_hops(t, &out.edge_indices, &dimmed_edges, &mut out.hop_data),
            None => (0, 0),
        };

        if self.last_status.len() == statuses.len() {
            if self.filters.only_problems {
                let is_problem =
                    |status: u8| status == HealthStatus::Degraded.as_u8() || status == HealthStatus::Failing.as_u8();
                let changed_problem_set = self.nodes.iter().enumerate().any(|(i, node)| {
                    node.kind == EntityKind::Service && is_problem(self.last_status[i]) != is_problem(statuses[i])
                });
                if changed_problem_set {
                    self.topology_revision += 1;
                }
            }
            for (i, (a, b)) in self.last_status.iter().zip(statuses.iter()).enumerate() {
                if a != b {
                    out.changed_nodes.push(i as u32);
                }
            }
        } else {
            out.changed_nodes = (0..statuses.len() as u32).collect();
        }
        self.last_status = statuses;

        out.summary = FrameSummary {
            generation: self.last_generation,
            time_ns: t,
            prev_event_ns: t.and_then(|t| self.timeline.as_ref().and_then(|tl| tl.prev_time(t))),
            next_event_ns: t.and_then(|t| self.timeline.as_ref().and_then(|tl| tl.next_time(t))),
            total_active: total_active.max(0) as u32,
            window_requests: win_req.max(0) as u32,
            window_errors: win_err.max(0) as u32,
            visible_nodes: out.node_indices.len() as u32,
            visible_edges: out.edge_indices.len() as u32,
            lod,
            topology_revision: self.topology_revision,
            hops_in_window,
            hops_shown,
        };
        out
    }

    /// Writes hops on `visible_edges` overlapping the hop window around `t`, with times
    /// relative to `t` in milliseconds (f32 keeps sub-microsecond precision within a minute).
    /// Returns `(overlapping, written)`.
    fn collect_hops(&mut self, t: i64, visible_edges: &[u32], dimmed: &HashSet<u32>, out: &mut Vec<f32>) -> (u32, u32) {
        if self.hops.is_none() {
            self.hops = Some(self.build_hops());
        }
        let Some(index) = self.hops.as_ref() else { return (0, 0) };
        let visible: HashSet<u32> = visible_edges.iter().copied().collect();
        let selected = self.filters.selected_trace_id.as_deref().map(|id| stable_hash(&[id]));
        let only = self.filters.isolated_trace().map(|id| stable_hash(&[id]));
        let (before, after) = self.hop_window;
        let max = self.config.max_hops as usize;
        let rel = |ns: i64| ((i128::from(ns) - i128::from(t)) as f64 / 1e6) as f32;
        let (mut total, mut shown) = (0u32, 0u32);
        for h in index.overlapping(t.saturating_sub(before), t.saturating_add(after)) {
            if !visible.contains(&h.edge) || only.is_some_and(|o| o != h.trace) {
                continue;
            }
            total += 1;
            if (shown as usize) < max {
                shown += 1;
                let mut flags = if selected == Some(h.trace) { h.flags | HOP_HIGHLIGHT } else { h.flags };
                if dimmed.contains(&h.edge) {
                    flags |= HOP_DIMMED;
                }
                out.extend_from_slice(&[
                    h.edge as f32,
                    rel(h.t[0]),
                    rel(h.t[1]),
                    rel(h.t[2]),
                    rel(h.t[3]),
                    flags as f32,
                    if h.origin == hops::NO_ORIGIN { -1.0 } else { h.origin as f32 },
                ]);
            }
        }
        (total, shown)
    }

    /// Hops from recorded spans, attributed to edges exactly like the timeline does.
    fn build_hops(&self) -> HopIndex {
        let mut hops = Vec::new();
        let mut skew = SkewCache::new();
        let roots = self.trace_roots();
        for s in self.spans.values() {
            let Some(&node) = self.node_index.get(&s.service_id) else { continue };
            let trace = stable_hash(&[&s.trace_id]);
            if let Some(&target) = s.target_entity_id.as_ref().and_then(|t| self.node_index.get(t)) {
                if let Some(&e) = self.edge_index.get(&(node, target, RelationKind::DependsOn)) {
                    let origin = self.trace_origin(s, &roots);
                    hops.push(Hop::leaf(e, s, trace, self.skew_ns(s, &mut skew)).with_origin(origin));
                }
                continue;
            }
            if let Some(src) = self.external_caller(s) {
                if let Some(&e) = self.edge_index.get(&(src, node, RelationKind::Calls)) {
                    let origin = self.trace_origin(s, &roots);
                    hops.push(Hop::inbound(e, s, trace, self.skew_ns(s, &mut skew)).with_origin(origin));
                }
                continue;
            }
            let (true, Some(parent)) = (s.kind == 2 || s.kind == 5, s.parent_span_id.as_deref()) else {
                continue;
            };
            let Some(p) = self.spans.get(&(s.trace_id.clone(), parent.to_owned())) else { continue };
            match self.node_index.get(&p.service_id) {
                Some(&caller) if caller != node => {
                    if let Some(&e) = self.edge_index.get(&(caller, node, RelationKind::Calls)) {
                        let shift = (self.skew_ns(p, &mut skew), self.skew_ns(s, &mut skew));
                        let origin = self.trace_origin(s, &roots);
                        hops.push(Hop::call(e, p, s, trace, shift).with_origin(origin));
                    }
                }
                _ => {}
            }
        }
        HopIndex::new(hops)
    }

    /// External client node that sent a root server span, when the topology has it.
    fn external_caller(&self, s: &SpanItem) -> Option<u32> {
        if s.kind != 2 || s.parent_span_id.is_some() {
            return None;
        }
        s.source_entity_id.as_ref().and_then(|id| self.node_index.get(id)).copied()
    }

    /// Service node of each loaded trace's root span (the earliest one if there are several).
    fn trace_roots(&self) -> HashMap<&str, (i64, u32)> {
        let mut roots: HashMap<&str, (i64, u32)> = HashMap::new();
        for s in self.spans.values().filter(|s| s.parent_span_id.is_none()) {
            let Some(&node) = self.node_index.get(&s.service_id) else { continue };
            let slot = roots.entry(s.trace_id.as_str()).or_insert((s.start_ns, node));
            if (s.start_ns, node) < *slot {
                *slot = (s.start_ns, node);
            }
        }
        roots
    }

    /// Service where the trace of `s` started: its root span's service or, when the root is not
    /// loaded, the service of the topmost loaded ancestor of `s`.
    fn trace_origin(&self, s: &SpanItem, roots: &HashMap<&str, (i64, u32)>) -> Option<u32> {
        const MAX_WALK: usize = 64;
        if let Some(&(_, node)) = roots.get(s.trace_id.as_str()) {
            return Some(node);
        }
        let mut cur = s;
        for _ in 0..MAX_WALK {
            let Some(parent) = cur.parent_span_id.as_deref() else { break };
            let Some(p) = self.spans.get(&(cur.trace_id.clone(), parent.to_owned())) else { break };
            cur = p;
        }
        self.node_index.get(&cur.service_id).copied()
    }

    /// Metrics for one node at an arbitrary time (defaults to the current time).
    pub fn node_metrics(&mut self, node_id: &str, at: Option<i64>) -> Option<NodeMetrics> {
        self.ensure_timeline();
        let idx = *self.node_index.get(node_id)? as usize;
        let t = at.or(self.current_time);
        let (now, past) = self.states_at(t);
        let window = self.window_for(&now, &past, true);
        let m = window.get(idx)?;
        let n = self.nodes.get(idx)?;
        Some(m.to_metrics(n))
    }

    /// Traffic over the edge `source_id → target_id` in the rolling window ending at `at`
    /// (defaults to the current time). `None` when no such edge exists.
    pub fn edge_metrics(&mut self, source_id: &str, target_id: &str, at: Option<i64>) -> Option<EdgeMetrics> {
        self.ensure_timeline();
        let s = *self.node_index.get(source_id)?;
        let t = *self.node_index.get(target_id)?;
        let (idx, rec) = self.edges.iter().enumerate().find(|(_, e)| e.source == s && e.target == t)?;
        let time = at.or(self.current_time).unwrap_or(0);
        let window = self.config.rolling_window_ns.max(1);
        let (now, past) = self.states_at(Some(time));
        let calls = (now.edge(idx, E_CALLS) - past.edge(idx, E_CALLS)).max(0);
        let errors = (now.edge(idx, E_ERRORS) - past.edge(idx, E_ERRORS)).max(0);
        let active = now.edge(idx, E_ACTIVE).max(0);

        let from = time.saturating_sub(window);
        let slice = |ts: i64| -> Option<usize> {
            (ts > from && ts <= time).then(|| {
                let off = i128::from(ts - from - 1) * EDGE_SERIES_BUCKETS as i128 / i128::from(window);
                (off.max(0) as usize).min(EDGE_SERIES_BUCKETS - 1)
            })
        };
        let mut calls_series = vec![0u32; EDGE_SERIES_BUCKETS];
        let mut errors_series = vec![0u32; EDGE_SERIES_BUCKETS];
        let mut durations: Vec<i64> = Vec::new();
        for span in self.spans.values() {
            let in_start = slice(span.start_ns);
            let in_end = slice(span.end_ns);
            if in_start.is_none() && in_end.is_none() {
                continue;
            }
            let Some(&node) = self.node_index.get(&span.service_id) else { continue };
            if !matches!(self.span_edge(span, node), EdgeLookup::Existing(e) if e as usize == idx) {
                continue;
            }
            if let Some(b) = in_start
                && let Some(c) = calls_series.get_mut(b)
            {
                *c += 1;
            }
            if let Some(b) = in_end {
                durations.push(span.end_ns.saturating_sub(span.start_ns).max(0));
                if span.is_error
                    && let Some(c) = errors_series.get_mut(b)
                {
                    *c += 1;
                }
            }
        }

        let status = classify_status(calls, errors, &self.config);
        let (status, status_reason) = match status {
            HealthStatus::Unknown if active > 0 => {
                (HealthStatus::Healthy, format!("{active} call(s) in flight, none started in window"))
            }
            HealthStatus::Unknown => (HealthStatus::Unknown, "no calls in window".into()),
            status => (status, classify_reason(calls, errors)),
        };
        let source = self.nodes.get(s as usize)?;
        let target = self.nodes.get(t as usize)?;
        Some(EdgeMetrics {
            source: source.id.clone(),
            source_name: source.name.clone(),
            target: target.id.clone(),
            target_name: target.name.clone(),
            relation: rec.relation.into(),
            provenance: rec.provenance.into(),
            reasons: rec.reasons.clone(),
            status: status.into(),
            status_reason,
            time_ns: time,
            window_ns: window,
            active: u32::try_from(active).unwrap_or(u32::MAX),
            window_calls: u32::try_from(calls).unwrap_or(u32::MAX),
            window_errors: u32::try_from(errors).unwrap_or(u32::MAX),
            calls_per_min: calls as f64 * 60e9 / window as f64,
            error_ratio: (calls > 0).then(|| errors as f64 / calls as f64),
            latency: latency_stats(&mut durations),
            calls_series,
            errors_series,
        })
    }

    /// Compare node state at two moments.
    pub fn compare(&mut self, a_ns: i64, b_ns: i64) -> Vec<NodeDiff> {
        self.ensure_timeline();
        let (na, pa) = self.states_at(Some(a_ns));
        let (nb, pb) = self.states_at(Some(b_ns));
        let wa = self.window_for(&na, &pa, true);
        let wb = self.window_for(&nb, &pb, true);
        self.nodes
            .iter()
            .zip(wa.iter().zip(wb.iter()))
            .filter(|(_, (a, b))| a.status != b.status || a.started != b.started || a.errors != b.errors)
            .map(|(n, (a, b))| NodeDiff {
                id: n.id.clone(),
                name: n.name.clone(),
                before: a.to_metrics(n),
                after: b.to_metrics(n),
            })
            .collect()
    }

    /// Hit-test in world coordinates.
    pub fn pick(&self, x: f32, y: f32) -> Option<String> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let (nx, ny) = self.pos(i, &n.id)?;
                let d2 = (nx - x).powi(2) + (ny - y).powi(2);
                (d2 <= 30.0f32.powi(2)).then_some((d2, n.id.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
            .or_else(|| {
                let node = self.infra_layout.as_ref()?.chip_at(x, y)?;
                self.nodes.get(node as usize).map(|n| n.id.clone())
            })
    }

    pub fn diagnostics(&mut self) -> Diagnostics {
        self.ensure_timeline();
        let tl = self.timeline.as_ref();
        let (first, last) = self.item_bounds();
        let items_bytes = self.spans.len() * 256 + self.logs.len() * 128 + self.events.len() * 128;
        Diagnostics {
            spans: self.spans.len() as u32,
            logs: self.logs.len() as u32,
            events: self.events.len() as u32,
            deltas: tl.map_or(0, |t| t.deltas.len() as u32),
            checkpoints: tl.map_or(0, |t| t.checkpoints.len() as u32),
            checkpoint_interval: tl.map_or(0, |t| t.interval as u32),
            nodes: self.nodes.len() as u32,
            edges: self.edges.len() as u32,
            approx_memory_bytes: (items_bytes + tl.map_or(0, |t| t.approx_bytes())) as u64,
            duplicates_ignored: self.duplicates_ignored,
            evicted_items: self.evicted_items,
            evicted_before_ns: self.evicted_before,
            unresolved_parents: self.unresolved_parents,
            stale_seeks_ignored: self.stale_seeks,
            timeline_rebuilds: self.timeline_rebuilds,
            timeline_merges: self.timeline_merges,
            first_item_ns: first,
            last_item_ns: last,
        }
    }

    pub fn item_count(&self) -> usize {
        self.spans.len() + self.logs.len() + self.events.len()
    }

    // ------------------------------------------------------------- internal

    /// Nodes drawn dimmed: muted services, and dependencies whose every caller is dimmed.
    pub(crate) fn dimmed_nodes(&self) -> Vec<bool> {
        let n = self.nodes.len();
        let mut dimmed = vec![false; n];
        if self.filters.muted_services.is_empty() {
            return dimmed;
        }
        let muted: HashSet<&str> = self.filters.muted_services.iter().map(String::as_str).collect();
        for (i, node) in self.nodes.iter().enumerate() {
            dimmed[i] = node.kind == EntityKind::Service && muted.contains(node.id.as_str());
        }
        let is_service = |i: usize| self.nodes.get(i).is_some_and(|x| x.kind == EntityKind::Service);
        let (mut called, mut live) = (vec![false; n], vec![false; n]);
        for e in &self.edges {
            let (s, t) = (e.source as usize, e.target as usize);
            if t < n && s < n && !is_service(t) {
                called[t] = true;
                live[t] |= !dimmed[s];
            }
        }
        for i in 0..n {
            if called[i] && !live[i] {
                dimmed[i] = true;
            }
        }
        dimmed
    }

    fn passes_filter(&self, idx: usize) -> bool {
        let Some(n) = self.nodes.get(idx) else { return false };
        if self.filters.hide_inferred && n.provenance == Provenance::Inferred {
            return false;
        }
        let search = &self.lowercase_search;
        search.is_empty() || n.lowercase_name.contains(search) || n.id.contains(search)
    }

    fn in_viewport(&self, x: f32, y: f32, r: f32) -> bool {
        match self.viewport {
            None => true,
            Some(v) if v.width <= 0.0 || v.height <= 0.0 => true,
            Some(v) => {
                let margin = r + 40.0;
                x + margin >= v.x && x - margin <= v.x + v.width && y + margin >= v.y && y - margin <= v.y + v.height
            }
        }
    }

    fn item_bounds(&self) -> (Option<i64>, Option<i64>) {
        let times = self
            .spans
            .values()
            .flat_map(|s| [s.start_ns, s.end_ns])
            .chain(self.logs.values().map(|l| l.time_ns))
            .chain(self.events.values().map(|e| e.time_ns));
        let mut first: Option<i64> = None;
        let mut last: Option<i64> = None;
        for t in times {
            first = Some(first.map_or(t, |f| f.min(t)));
            last = Some(last.map_or(t, |l| l.max(t)));
        }
        (first, last)
    }

    fn ensure_node(&mut self, id: &str, name: &str, kind: EntityKind, provenance: Provenance) -> u32 {
        if let Some(&i) = self.node_index.get(id) {
            return i;
        }
        let i = self.nodes.len() as u32;
        self.nodes.push(NodeRec {
            id: id.to_owned(),
            name: name.to_owned(),
            lowercase_name: name.to_lowercase(),
            kind,
            provenance,
        });
        self.node_index.insert(id.to_owned(), i);
        self.topology_revision += 1;
        self.layout_dirty = true;
        i
    }

    fn ensure_edge(&mut self, s: u32, t: u32, relation: RelationKind, provenance: Provenance) -> u32 {
        if let Some(&i) = self.edge_index.get(&(s, t, relation)) {
            return i;
        }
        let i = self.edges.len() as u32;
        self.edges.push(EdgeRec { source: s, target: t, relation, provenance, reasons: Vec::new() });
        self.edge_index.insert((s, t, relation), i);
        self.topology_revision += 1;
        self.layout_dirty = true;
        i
    }

    /// Recompute the layout if nodes or edges were added since the last one.
    fn relayout(&mut self) {
        if !self.layout_dirty {
            return;
        }
        self.layout_dirty = false;
        let ids: Vec<String> = self.nodes.iter().map(|n| n.id.clone()).collect();
        let edges: Vec<(u32, u32)> = self.edges.iter().map(|e| (e.source, e.target)).collect();
        self.layout.compute(&ids, &edges);
        self.refresh_infra();
        self.topology_revision += 1;
    }

    fn evict_if_needed(&mut self) {
        let max = self.config.max_items as usize;
        let count = self.item_count();
        if count <= max {
            return;
        }
        let mut times: Vec<i64> = self
            .spans
            .values()
            .map(|s| s.end_ns)
            .chain(self.logs.values().map(|l| l.time_ns))
            .chain(self.events.values().map(|e| e.time_ns))
            .collect();
        let excess = count - max;
        let nth = excess.saturating_sub(1).min(times.len().saturating_sub(1));
        times.select_nth_unstable(nth);
        let cutoff = times.get(nth).copied().unwrap_or(i64::MIN).saturating_add(1);
        self.evict_before(cutoff);
    }

    /// Edge a span contributes to, resolved the same way as in a full rebuild.
    fn span_edge(&self, s: &SpanItem, node: u32) -> EdgeLookup {
        if let Some(&target) = s.target_entity_id.as_ref().and_then(|t| self.node_index.get(t)) {
            return self
                .edge_index
                .get(&(node, target, RelationKind::DependsOn))
                .map_or(EdgeLookup::Missing, |&e| EdgeLookup::Existing(e));
        }
        if let Some(src) = self.external_caller(s) {
            return self
                .edge_index
                .get(&(src, node, RelationKind::Calls))
                .map_or(EdgeLookup::Missing, |&e| EdgeLookup::Existing(e));
        }
        let (true, Some(parent)) = (s.kind == 2 || s.kind == 5, s.parent_span_id.as_deref()) else {
            return EdgeLookup::NoEdge;
        };
        let parent_key = (s.trace_id.clone(), parent.to_owned());
        match self.spans.get(&parent_key).and_then(|p| self.node_index.get(&p.service_id)) {
            Some(&caller) if caller != node => self
                .edge_index
                .get(&(caller, node, RelationKind::Calls))
                .map_or(EdgeLookup::Missing, |&e| EdgeLookup::Existing(e)),
            Some(_) => EdgeLookup::NoEdge,
            None => EdgeLookup::Unresolved(parent_key),
        }
    }

    /// Merge pending items into the existing timeline. Returns false when the change is
    /// structural (new edge, a late parent resolving an earlier child, budget exceeded) and a
    /// full rebuild is required.
    fn try_merge_pending(&mut self) -> bool {
        let pending = std::mem::take(&mut self.pending);
        let mut deltas = Vec::with_capacity(pending.len() * 2);
        let mut new_unresolved = Vec::new();
        for key in &pending {
            match key {
                ItemKey::Span(k) => {
                    let Some(s) = self.spans.get(k) else { continue };
                    if self.unresolved_keys.contains(k) {
                        return false;
                    }
                    let Some(&node) = self.node_index.get(&s.service_id) else { continue };
                    let edge = match self.span_edge(s, node) {
                        EdgeLookup::NoEdge => None,
                        EdgeLookup::Existing(e) => Some(e),
                        EdgeLookup::Missing => return false,
                        EdgeLookup::Unresolved(pk) => {
                            new_unresolved.push(pk);
                            None
                        }
                    };
                    push_span_deltas(&mut deltas, s, node, edge);
                }
                ItemKey::Log(id) => {
                    let Some(l) = self.logs.get(id) else { continue };
                    let Some(&node) = self.node_index.get(&l.service_id) else { continue };
                    deltas.push(log_delta(l, node));
                }
                ItemKey::Event(id) => {
                    let Some(e) = self.events.get(id) else { continue };
                    let node = e.service_id.as_ref().and_then(|s| self.node_index.get(s)).copied();
                    deltas.push(event_delta(e, node));
                }
            }
        }
        let Some(tl) = self.timeline.as_mut() else { return false };
        tl.merge(deltas);
        if tl.over_budget() {
            return false;
        }
        self.unresolved_parents += new_unresolved.len() as u32;
        self.unresolved_keys.extend(new_unresolved);
        self.timeline_merges += 1;
        true
    }

    /// Bring ordered deltas + checkpoints up to date: merge new items when possible,
    /// otherwise rebuild from all recorded items.
    fn ensure_timeline(&mut self) {
        self.build_timeline();
        // Building the timeline discovers edges from spans; lay them out before the next frame.
        self.relayout();
    }

    fn build_timeline(&mut self) {
        if self.timeline.is_some() && !self.pending.is_empty() && !self.try_merge_pending() {
            self.timeline = None;
        }
        if self.timeline.is_some() {
            return;
        }
        self.pending.clear();
        self.unresolved_keys.clear();
        self.hops = None;
        self.timeline_rebuilds += 1;
        let span_service: HashMap<(&str, &str), u32> = self
            .spans
            .values()
            .filter_map(|s| self.node_index.get(&s.service_id).map(|&n| ((s.trace_id.as_str(), s.span_id.as_str()), n)))
            .collect();

        /// (trace id, span id), from node, to node, relation, provenance.
        type SpanEdge = ((String, String), u32, u32, RelationKind, Provenance);
        let mut span_edges: Vec<SpanEdge> = Vec::new();
        let mut unresolved = 0u32;
        let mut unresolved_keys = HashSet::new();
        for s in self.spans.values() {
            let Some(&node) = self.node_index.get(&s.service_id) else { continue };
            let key = (s.trace_id.clone(), s.span_id.clone());
            if let Some(&target) = s.target_entity_id.as_ref().and_then(|t| self.node_index.get(t)) {
                span_edges.push((key, node, target, RelationKind::DependsOn, Provenance::Inferred));
            } else if let Some(src) = self.external_caller(s) {
                span_edges.push((key, src, node, RelationKind::Calls, Provenance::Inferred));
            } else if (s.kind == 2 || s.kind == 5)
                && let Some(parent) = s.parent_span_id.as_deref()
            {
                match span_service.get(&(s.trace_id.as_str(), parent)) {
                    Some(&caller) if caller != node => {
                        span_edges.push((key, caller, node, RelationKind::Calls, Provenance::Observed));
                    }
                    Some(_) => {}
                    None => {
                        unresolved += 1;
                        unresolved_keys.insert((s.trace_id.clone(), parent.to_owned()));
                    }
                }
            }
        }
        // Sort so edge indices are assigned deterministically regardless of HashMap order.
        span_edges.sort_by(|a, b| a.0.cmp(&b.0));
        let mut edge_of_span: HashMap<(String, String), u32> = HashMap::new();
        for (key, a, b, rel, prov) in span_edges {
            let idx = match self.edge_index.get(&(a, b, rel)) {
                Some(&i) => i,
                None => {
                    if self.server_geometry.is_some() || self.config.server_owned_topology {
                        continue;
                    }
                    let i = self.ensure_edge(a, b, rel, prov);
                    if prov == Provenance::Inferred
                        && let Some(e) = self.edges.get_mut(i as usize)
                    {
                        e.reasons.push(if rel == RelationKind::DependsOn {
                            "client span references this resource (db.system / peer attributes)".into()
                        } else {
                            "server span has no instrumented caller: the request came from outside the project".into()
                        });
                    }
                    i
                }
            };
            edge_of_span.insert(key, idx);
        }
        self.unresolved_parents = unresolved;
        self.unresolved_keys = unresolved_keys;

        let mut deltas = Vec::with_capacity(self.spans.len() * 2 + self.logs.len() + self.events.len());
        for s in self.spans.values() {
            let Some(&node) = self.node_index.get(&s.service_id) else { continue };
            let key = (s.trace_id.clone(), s.span_id.clone());
            push_span_deltas(&mut deltas, s, node, edge_of_span.get(&key).copied());
        }
        for l in self.logs.values() {
            let Some(&node) = self.node_index.get(&l.service_id) else { continue };
            deltas.push(log_delta(l, node));
        }
        for e in self.events.values() {
            let node = e.service_id.as_ref().and_then(|s| self.node_index.get(s)).copied();
            deltas.push(event_delta(e, node));
        }
        self.timeline = Some(Timeline::build(
            deltas,
            self.nodes.len(),
            self.edges.len(),
            self.config.checkpoint_budget_bytes as usize,
        ));
    }

    fn states_at(&self, t: Option<i64>) -> (StateVec, StateVec) {
        match (t, self.timeline.as_ref()) {
            (Some(t), Some(tl)) => {
                (tl.state_at(t), tl.state_at(t.saturating_sub(self.config.rolling_window_ns.max(0))))
            }
            _ => (StateVec::new(self.nodes.len(), self.edges.len()), StateVec::new(self.nodes.len(), self.edges.len())),
        }
    }

    fn window_for(&self, now: &StateVec, past: &StateVec, include_reasons: bool) -> Vec<WindowMetrics> {
        let d = |n: usize, f: usize| now.node(n, f).wrapping_sub(past.node(n, f));
        let mut incoming: Vec<(i64, i64, i64)> = vec![(0, 0, 0); self.nodes.len()];
        let mut outgoing: Vec<(i64, i64, i64)> = vec![(0, 0, 0); self.nodes.len()];
        for (i, e) in self.edges.iter().enumerate() {
            let m = (
                now.edge(i, E_CALLS) - past.edge(i, E_CALLS),
                now.edge(i, E_ERRORS) - past.edge(i, E_ERRORS),
                now.edge(i, E_ACTIVE),
            );
            for slot in [incoming.get_mut(e.target as usize), outgoing.get_mut(e.source as usize)].into_iter().flatten()
            {
                slot.0 += m.0;
                slot.1 += m.1;
                slot.2 += m.2;
            }
        }
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let mut m = WindowMetrics {
                    active: now.node(i, N_ACTIVE).max(0),
                    started: d(i, N_STARTED),
                    completed: d(i, N_COMPLETED),
                    errors: d(i, N_ERRORS),
                    latency_sum: d(i, N_LATENCY_SUM),
                    logs: d(i, N_LOGS),
                    error_logs: d(i, N_ERROR_LOGS),
                    events: d(i, N_EVENTS),
                    any_spans: d(i, N_ANY_SPANS),
                    status: HealthStatus::Unknown,
                    reason: String::new(),
                };
                // An external client only calls; everything else that is not a service is called.
                let client = n.kind == EntityKind::ExternalClient;
                let side = if client { &outgoing } else { &incoming };
                let (calls_in, errors_in, active_in) = side.get(i).copied().unwrap_or((0, 0, 0));
                if n.kind != EntityKind::Service {
                    m.active = active_in;
                    m.started = calls_in;
                    m.completed = calls_in;
                    m.errors = errors_in;
                    m.status = classify_status(calls_in, errors_in, &self.config);
                    if include_reasons {
                        if m.status != HealthStatus::Unknown {
                            m.reason.push_str(&classify_reason(calls_in, errors_in));
                            m.reason.push_str(if client {
                                " — derived from spans of the services it calls; the client itself is not instrumented"
                            } else {
                                " — derived from caller spans; resource itself is not instrumented"
                            });
                        } else if client {
                            m.reason = "not instrumented; no requests from it in window".into();
                        } else {
                            m.reason = "not instrumented; no caller activity in window".into();
                        }
                    }
                } else if m.completed > 0 {
                    m.status = classify_status(m.completed, m.errors, &self.config);
                    if include_reasons {
                        m.reason = classify_reason(m.completed, m.errors);
                    }
                } else if m.active > 0 {
                    m.status = HealthStatus::Healthy;
                    if include_reasons {
                        m.reason = format!("{} request(s) in flight, none completed in window", m.active);
                    }
                } else if m.error_logs > 0 {
                    m.status = HealthStatus::Degraded;
                    if include_reasons {
                        m.reason = format!("{} error log(s) in window, no completed requests", m.error_logs);
                    }
                } else if include_reasons {
                    if m.logs > 0 || m.any_spans > 0 || m.events > 0 {
                        m.reason = "activity seen but no request spans completed in window".into();
                    } else {
                        m.reason = "no data in window".into();
                    }
                }
                m
            })
            .collect()
    }

    fn trace_highlight(&self) -> (HashSet<u32>, HashSet<u32>) {
        let mut nodes = HashSet::new();
        let mut edges = HashSet::new();
        let Some(trace) = self.filters.selected_trace_id.as_deref() else {
            return (nodes, edges);
        };
        let spans: Vec<&SpanItem> = self.spans.values().filter(|s| s.trace_id == trace).collect();
        let by_id: HashMap<&str, &SpanItem> = spans.iter().map(|s| (s.span_id.as_str(), *s)).collect();
        for s in &spans {
            let Some(&n) = self.node_index.get(&s.service_id) else { continue };
            nodes.insert(n);
            if let Some(t) = s.target_entity_id.as_ref().and_then(|t| self.node_index.get(t)) {
                nodes.insert(*t);
                if let Some(&e) = self.edge_index.get(&(n, *t, RelationKind::DependsOn)) {
                    edges.insert(e);
                }
            }
            if let Some(src) = self.external_caller(s) {
                nodes.insert(src);
                if let Some(&e) = self.edge_index.get(&(src, n, RelationKind::Calls)) {
                    edges.insert(e);
                }
            }
            if let Some(p) = s.parent_span_id.as_deref().and_then(|p| by_id.get(p))
                && let Some(&pn) = self.node_index.get(&p.service_id)
                && let Some(&e) = self.edge_index.get(&(pn, n, RelationKind::Calls))
            {
                edges.insert(e);
            }
        }
        (nodes, edges)
    }
}

#[derive(Debug, Clone)]
struct WindowMetrics {
    active: i64,
    started: i64,
    completed: i64,
    errors: i64,
    latency_sum: i64,
    logs: i64,
    error_logs: i64,
    events: i64,
    any_spans: i64,
    status: HealthStatus,
    reason: String,
}

impl WindowMetrics {
    fn to_metrics(&self, n: &NodeRec) -> NodeMetrics {
        NodeMetrics {
            id: n.id.clone(),
            name: n.name.clone(),
            status: self.status.into(),
            status_reason: self.reason.clone(),
            active: self.active.max(0) as u32,
            window_requests: self.started.max(0) as u32,
            window_completed: self.completed.max(0) as u32,
            window_errors: self.errors.max(0) as u32,
            avg_latency_ms: (self.completed > 0 && n.kind == EntityKind::Service)
                .then(|| self.latency_sum as f64 / self.completed as f64 / 1e6),
            window_logs: self.logs.max(0) as u32,
            window_error_logs: self.error_logs.max(0) as u32,
            window_events: self.events.max(0) as u32,
        }
    }
}

/// Nearest-rank percentiles over call durations (ns); `None` for an empty set.
fn latency_stats(durations: &mut [i64]) -> Option<EdgeLatency> {
    if durations.is_empty() {
        return None;
    }
    durations.sort_unstable();
    let n = durations.len();
    let rank = |p: f64| -> f64 {
        let i = ((p * n as f64).ceil() as usize).clamp(1, n) - 1;
        durations.get(i).copied().unwrap_or(0) as f64 / 1e6
    };
    let sum: i128 = durations.iter().map(|&d| i128::from(d)).sum();
    Some(EdgeLatency {
        samples: u32::try_from(n).unwrap_or(u32::MAX),
        avg_ms: sum as f64 / n as f64 / 1e6,
        p50_ms: rank(0.5),
        p95_ms: rank(0.95),
        max_ms: durations.last().copied().unwrap_or(0) as f64 / 1e6,
    })
}

fn classify_status(completed: i64, errors: i64, cfg: &EngineConfig) -> HealthStatus {
    if completed <= 0 {
        return HealthStatus::Unknown;
    }
    let ratio = errors as f32 / completed as f32;
    if ratio >= cfg.failing_error_ratio {
        HealthStatus::Failing
    } else if ratio >= cfg.degraded_error_ratio {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    }
}

fn classify_reason(completed: i64, errors: i64) -> String {
    if completed <= 0 {
        return String::new();
    }
    let ratio = errors as f32 / completed as f32;
    format!("{errors}/{completed} failed ({:.1}%)", ratio * 100.0)
}

#[cfg(test)]
mod robustness_tests;
#[cfg(test)]
mod tests;
