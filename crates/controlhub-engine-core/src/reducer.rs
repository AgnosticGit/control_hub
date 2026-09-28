//! Deterministic reducer: `state(t) = checkpoint + ordered deltas up to t`.
//!
//! All deltas are additive counters, so the state at `t` depends only on the
//! *set* of deltas with `time <= t`, never on arrival order. That makes late,
//! out-of-order and duplicated input converge to the same result, and lets
//! "step backwards" be a plain seek (restore nearest checkpoint + replay)
//! without requiring inverse operations.

pub(crate) const NODE_FIELDS: usize = 9;
pub(crate) const EDGE_FIELDS: usize = 3;

// node field offsets
pub(crate) const N_ACTIVE: usize = 0;
pub(crate) const N_STARTED: usize = 1;
pub(crate) const N_COMPLETED: usize = 2;
pub(crate) const N_ERRORS: usize = 3;
pub(crate) const N_LATENCY_SUM: usize = 4;
pub(crate) const N_LOGS: usize = 5;
pub(crate) const N_ERROR_LOGS: usize = 6;
pub(crate) const N_EVENTS: usize = 7;
pub(crate) const N_ANY_SPANS: usize = 8;
// edge field offsets
pub(crate) const E_ACTIVE: usize = 0;
pub(crate) const E_CALLS: usize = 1;
pub(crate) const E_ERRORS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DeltaKind {
    SpanStart { node: u32, edge: Option<u32>, entry: bool },
    SpanEnd { node: u32, edge: Option<u32>, entry: bool, error: bool, duration_ns: i64 },
    Log { node: u32, error: bool },
    Event { node: Option<u32> },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Delta {
    pub time: i64,
    pub tiebreak: u64,
    pub kind: DeltaKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StateVec {
    pub nodes: usize,
    pub data: Vec<i64>,
}

impl StateVec {
    pub fn new(nodes: usize, edges: usize) -> Self {
        Self { nodes, data: vec![0; nodes * NODE_FIELDS + edges * EDGE_FIELDS] }
    }

    #[inline]
    pub fn node(&self, n: usize, f: usize) -> i64 {
        self.data.get(n * NODE_FIELDS + f).copied().unwrap_or(0)
    }

    #[inline]
    pub fn edge(&self, e: usize, f: usize) -> i64 {
        self.data.get(self.nodes * NODE_FIELDS + e * EDGE_FIELDS + f).copied().unwrap_or(0)
    }

    #[inline]
    fn add_node(&mut self, n: u32, f: usize, v: i64) {
        if let Some(slot) = self.data.get_mut(n as usize * NODE_FIELDS + f) {
            *slot += v;
        }
    }

    #[inline]
    fn add_edge(&mut self, e: u32, f: usize, v: i64) {
        let idx = self.nodes * NODE_FIELDS + e as usize * EDGE_FIELDS + f;
        if let Some(slot) = self.data.get_mut(idx) {
            *slot += v;
        }
    }

    pub fn apply(&mut self, d: &DeltaKind) {
        match *d {
            DeltaKind::SpanStart { node, edge, entry } => {
                if entry {
                    self.add_node(node, N_ACTIVE, 1);
                    self.add_node(node, N_STARTED, 1);
                }
                if let Some(e) = edge {
                    self.add_edge(e, E_ACTIVE, 1);
                    self.add_edge(e, E_CALLS, 1);
                }
            }
            DeltaKind::SpanEnd { node, edge, entry, error, duration_ns } => {
                self.add_node(node, N_ANY_SPANS, 1);
                if entry {
                    self.add_node(node, N_ACTIVE, -1);
                    self.add_node(node, N_COMPLETED, 1);
                    self.add_node(node, N_LATENCY_SUM, duration_ns.max(0));
                    if error {
                        self.add_node(node, N_ERRORS, 1);
                    }
                }
                if let Some(e) = edge {
                    self.add_edge(e, E_ACTIVE, -1);
                    if error {
                        self.add_edge(e, E_ERRORS, 1);
                    }
                }
            }
            DeltaKind::Log { node, error } => {
                self.add_node(node, N_LOGS, 1);
                if error {
                    self.add_node(node, N_ERROR_LOGS, 1);
                }
            }
            DeltaKind::Event { node } => {
                if let Some(n) = node {
                    self.add_node(n, N_EVENTS, 1);
                }
            }
        }
    }
}

/// Sorted deltas + periodic checkpoints.
#[derive(Debug, Clone)]
pub(crate) struct Timeline {
    pub deltas: Vec<Delta>,
    /// `checkpoints[i]` = state after applying `deltas[..(i + 1) * interval]`.
    pub checkpoints: Vec<StateVec>,
    pub interval: usize,
    pub nodes: usize,
    pub edges: usize,
}

impl Timeline {
    pub fn build(mut deltas: Vec<Delta>, nodes: usize, edges: usize, budget_bytes: usize) -> Self {
        deltas.sort_by(|a, b| a.time.cmp(&b.time).then(a.tiebreak.cmp(&b.tiebreak)));
        let state_bytes = (nodes * NODE_FIELDS + edges * EDGE_FIELDS).max(1) * 8;
        let max_checkpoints = (budget_bytes / state_bytes).max(1);
        let interval = (deltas.len() / max_checkpoints + 1).max(512);
        let mut checkpoints = Vec::with_capacity(deltas.len() / interval + 1);
        let mut state = StateVec::new(nodes, edges);
        for (i, d) in deltas.iter().enumerate() {
            state.apply(&d.kind);
            if (i + 1) % interval == 0 {
                checkpoints.push(state.clone());
            }
        }
        Self { deltas, checkpoints, interval, nodes, edges }
    }

    /// Number of deltas with `time <= t`.
    pub fn count_at(&self, t: i64) -> usize {
        self.deltas.partition_point(|d| d.time <= t)
    }

    pub fn state_at(&self, t: i64) -> StateVec {
        let upto = self.count_at(t);
        let cp = upto / self.interval;
        let (mut state, from) = if cp == 0 {
            (StateVec::new(self.nodes, self.edges), 0)
        } else {
            match self.checkpoints.get(cp - 1) {
                Some(s) => (s.clone(), cp * self.interval),
                None => (StateVec::new(self.nodes, self.edges), 0),
            }
        };
        for d in self.deltas.get(from..upto).unwrap_or(&[]) {
            state.apply(&d.kind);
        }
        state
    }

    pub fn prev_time(&self, t: i64) -> Option<i64> {
        let idx = self.deltas.partition_point(|d| d.time < t);
        idx.checked_sub(1).and_then(|i| self.deltas.get(i)).map(|d| d.time)
    }

    pub fn next_time(&self, t: i64) -> Option<i64> {
        let idx = self.deltas.partition_point(|d| d.time <= t);
        self.deltas.get(idx).map(|d| d.time)
    }

    pub fn approx_bytes(&self) -> usize {
        self.deltas.len() * std::mem::size_of::<Delta>()
            + self.checkpoints.iter().map(|c| c.data.len() * 8).sum::<usize>()
    }
}
