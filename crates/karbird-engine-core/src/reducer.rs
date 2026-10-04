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
    /// Time used by eviction for the item that produced this delta (span end, log/event time).
    /// Both deltas of a span share it, so a span is always evicted as a whole.
    pub evict_at: i64,
    pub kind: DeltaKind,
}

impl Delta {
    #[inline]
    fn order_key(&self) -> (i64, u64) {
        (self.time, self.tiebreak)
    }
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
        if n >= self.nodes {
            return 0;
        }
        self.data.get(n * NODE_FIELDS + f).copied().unwrap_or(0)
    }

    #[inline]
    pub fn edge(&self, e: usize, f: usize) -> i64 {
        self.data.get(self.nodes * NODE_FIELDS + e * EDGE_FIELDS + f).copied().unwrap_or(0)
    }

    /// Out-of-range nodes are ignored (never spill into the edge section). Sums wrap so that
    /// differences between two states stay exact even for absurd span durations.
    #[inline]
    fn add_node(&mut self, n: u32, f: usize, v: i64) {
        if n as usize >= self.nodes {
            return;
        }
        if let Some(slot) = self.data.get_mut(n as usize * NODE_FIELDS + f) {
            *slot = slot.wrapping_add(v);
        }
    }

    #[inline]
    fn add_edge(&mut self, e: u32, f: usize, v: i64) {
        let idx = self.nodes * NODE_FIELDS + e as usize * EDGE_FIELDS + f;
        if let Some(slot) = self.data.get_mut(idx) {
            *slot = slot.wrapping_add(v);
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
    /// Checkpoint count the memory budget allows; exceeding it twice over calls for a rebuild.
    pub max_checkpoints: usize,
}

impl Timeline {
    pub fn build(mut deltas: Vec<Delta>, nodes: usize, edges: usize, budget_bytes: usize) -> Self {
        deltas.sort_by_key(Delta::order_key);
        let state_bytes = (nodes * NODE_FIELDS + edges * EDGE_FIELDS).max(1) * 8;
        let max_checkpoints = (budget_bytes / state_bytes).max(1);
        let interval = (deltas.len() / max_checkpoints + 1).max(512);
        let mut tl = Self { deltas, checkpoints: Vec::new(), interval, nodes, edges, max_checkpoints };
        tl.recompute_checkpoints_from(0);
        tl
    }

    /// Recompute checkpoints `cp..`; checkpoints before `cp` must still be valid.
    fn recompute_checkpoints_from(&mut self, cp: usize) {
        let cp = cp.min(self.checkpoints.len());
        self.checkpoints.truncate(cp);
        let mut state = match cp.checked_sub(1).and_then(|i| self.checkpoints.get(i)) {
            Some(s) => s.clone(),
            None => StateVec::new(self.nodes, self.edges),
        };
        let start = cp * self.interval;
        for (i, d) in self.deltas.iter().enumerate().skip(start) {
            state.apply(&d.kind);
            if (i + 1) % self.interval == 0 {
                self.checkpoints.push(state.clone());
            }
        }
    }

    /// Insert new deltas (for already known nodes/edges). Only checkpoints at or after the
    /// earliest inserted delta are recomputed, so appending live data is cheap.
    pub fn merge(&mut self, mut new: Vec<Delta>) {
        new.sort_by_key(Delta::order_key);
        let Some(first) = new.first().map(Delta::order_key) else { return };
        let p = self.deltas.partition_point(|d| d.order_key() < first);
        let tail = self.deltas.split_off(p);
        self.deltas.reserve(tail.len() + new.len());
        let (mut a, mut b) = (tail.into_iter().peekable(), new.into_iter().peekable());
        loop {
            let take_a = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => x.order_key() <= y.order_key(),
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let next = if take_a { a.next() } else { b.next() };
            if let Some(d) = next {
                self.deltas.push(d);
            }
        }
        self.recompute_checkpoints_from(p / self.interval);
    }

    /// Drop deltas of items evicted before `before` and recompute checkpoints.
    pub fn retain_from(&mut self, before: i64) {
        let n0 = self.deltas.len();
        self.deltas.retain(|d| d.evict_at >= before);
        if self.deltas.len() != n0 {
            self.recompute_checkpoints_from(0);
        }
    }

    pub fn over_budget(&self) -> bool {
        self.checkpoints.len() > self.max_checkpoints.saturating_mul(2)
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use proptest::prelude::*;

    const NODES: usize = 3;
    const EDGES: usize = 2;

    fn arb_kind() -> impl Strategy<Value = DeltaKind> {
        let node = 0u32..NODES as u32;
        let edge = prop::option::of(0u32..EDGES as u32);
        prop_oneof![
            (node.clone(), edge.clone(), any::<bool>()).prop_map(|(node, edge, entry)| DeltaKind::SpanStart {
                node,
                edge,
                entry
            }),
            (node.clone(), edge, any::<bool>(), any::<bool>(), -5i64..1_000).prop_map(
                |(node, edge, entry, error, duration_ns)| DeltaKind::SpanEnd { node, edge, entry, error, duration_ns }
            ),
            (node.clone(), any::<bool>()).prop_map(|(node, error)| DeltaKind::Log { node, error }),
            prop::option::of(node).prop_map(|node| DeltaKind::Event { node }),
        ]
    }

    /// Few distinct times and tiebreaks, so equal order keys are common.
    fn arb_deltas(max: usize) -> impl Strategy<Value = Vec<Delta>> {
        prop::collection::vec(
            (0i64..200, 0u64..4, 0i64..200, arb_kind()).prop_map(|(time, tiebreak, evict_at, kind)| Delta {
                time,
                tiebreak,
                evict_at,
                kind,
            }),
            0..max,
        )
    }

    fn naive(deltas: &[Delta], t: i64) -> StateVec {
        let mut s = StateVec::new(NODES, EDGES);
        for d in deltas.iter().filter(|d| d.time <= t) {
            s.apply(&d.kind);
        }
        s
    }

    fn checkpoints_are_prefix_states(tl: &Timeline) -> Result<(), TestCaseError> {
        prop_assert_eq!(tl.checkpoints.len(), tl.deltas.len() / tl.interval);
        let mut s = StateVec::new(tl.nodes, tl.edges);
        for (i, d) in tl.deltas.iter().enumerate() {
            s.apply(&d.kind);
            if (i + 1) % tl.interval == 0 {
                prop_assert_eq!(&tl.checkpoints[i / tl.interval], &s, "checkpoint {}", i / tl.interval);
            }
        }
        prop_assert!(tl.deltas.windows(2).all(|w| w[0].order_key() <= w[1].order_key()), "deltas sorted");
        Ok(())
    }

    fn budget() -> impl Strategy<Value = usize> {
        prop::sample::select(vec![1usize, 200, 4_000, 16 << 20])
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn state_at_matches_naive(deltas in arb_deltas(1_600), budget in budget(), ts in prop::collection::vec(-1i64..210, 1..8)) {
            let tl = Timeline::build(deltas.clone(), NODES, EDGES, budget);
            checkpoints_are_prefix_states(&tl)?;
            for t in ts {
                prop_assert_eq!(tl.state_at(t), naive(&deltas, t), "t={}", t);
                prop_assert_eq!(tl.count_at(t), deltas.iter().filter(|d| d.time <= t).count());
            }
        }

        /// Merging in chunks produces exactly the deltas (and states) of one build.
        #[test]
        fn merge_equals_build(first in arb_deltas(800), rest in prop::collection::vec(arb_deltas(400), 0..4), budget in budget(), t in -1i64..210) {
            let mut tl = Timeline::build(first.clone(), NODES, EDGES, budget);
            let mut all = first;
            for chunk in rest {
                tl.merge(chunk.clone());
                all.extend(chunk);
                checkpoints_are_prefix_states(&tl)?;
            }
            let full = Timeline::build(all.clone(), NODES, EDGES, budget);
            prop_assert_eq!(&tl.deltas, &full.deltas);
            prop_assert_eq!(tl.state_at(t), naive(&all, t));
        }

        #[test]
        fn retain_from_equals_filtered_build(deltas in arb_deltas(1_200), before in -1i64..210, budget in budget(), t in -1i64..210) {
            let mut tl = Timeline::build(deltas.clone(), NODES, EDGES, budget);
            tl.retain_from(before);
            checkpoints_are_prefix_states(&tl)?;
            let kept: Vec<Delta> = deltas.into_iter().filter(|d| d.evict_at >= before).collect();
            prop_assert_eq!(tl.deltas.len(), kept.len());
            prop_assert_eq!(tl.state_at(t), naive(&kept, t));
        }

        #[test]
        fn prev_and_next_time_match_naive(deltas in arb_deltas(300), t in -1i64..210) {
            let tl = Timeline::build(deltas.clone(), NODES, EDGES, 16 << 20);
            let prev = deltas.iter().map(|d| d.time).filter(|&x| x < t).max();
            let next = deltas.iter().map(|d| d.time).filter(|&x| x > t).min();
            prop_assert_eq!(tl.prev_time(t), prev);
            prop_assert_eq!(tl.next_time(t), next);
        }

        /// State depends only on the set of deltas, not on their input order.
        #[test]
        fn input_order_is_irrelevant(deltas in arb_deltas(700), t in -1i64..210) {
            let mut reversed = deltas.clone();
            reversed.reverse();
            let a = Timeline::build(deltas, NODES, EDGES, 1);
            let b = Timeline::build(reversed, NODES, EDGES, 1);
            prop_assert_eq!(a.state_at(t), b.state_at(t));
        }
    }

    #[test]
    fn empty_timeline() {
        let tl = Timeline::build(Vec::new(), NODES, EDGES, 1);
        assert_eq!(tl.state_at(i64::MAX), StateVec::new(NODES, EDGES));
        assert_eq!((tl.prev_time(0), tl.next_time(0)), (None, None));
        assert!(tl.checkpoints.is_empty() && !tl.over_budget());
        let mut tl = tl;
        tl.merge(Vec::new());
        tl.retain_from(i64::MAX);
        assert!(tl.deltas.is_empty());
    }

    #[test]
    fn growing_past_twice_the_budget_is_reported() {
        let one_state = NODE_FIELDS * 8;
        let mut tl = Timeline::build(Vec::new(), 1, 0, one_state);
        assert_eq!((tl.max_checkpoints, tl.interval), (1, 512));
        let delta =
            |i: i64| Delta { time: i, tiebreak: 0, evict_at: i, kind: DeltaKind::Log { node: 0, error: false } };
        tl.merge((0..1_024).map(delta).collect());
        assert_eq!(tl.checkpoints.len(), 2);
        assert!(!tl.over_budget());
        tl.merge((1_024..1_536).map(delta).collect());
        assert!(tl.over_budget());
        assert_eq!(tl.state_at(i64::MAX).node(0, N_LOGS), 1_536);
    }

    #[test]
    fn out_of_range_indices_are_ignored() {
        let mut s = StateVec::new(1, 2);
        s.apply(&DeltaKind::SpanStart { node: 1, edge: Some(2), entry: true });
        s.apply(&DeltaKind::Log { node: u32::MAX, error: true });
        s.apply(&DeltaKind::SpanEnd { node: 7, edge: Some(u32::MAX), entry: true, error: true, duration_ns: 5 });
        assert!(s.data.iter().all(|&v| v == 0), "a node past the node section must not touch edge counters");
        assert_eq!((s.node(1, 0), s.edge(2, 0), s.node(usize::MAX / 16, 0)), (0, 0, 0));
    }

    #[test]
    fn span_deltas_update_the_right_counters() {
        let mut s = StateVec::new(2, 1);
        s.apply(&DeltaKind::SpanStart { node: 0, edge: Some(0), entry: true });
        s.apply(&DeltaKind::SpanStart { node: 1, edge: None, entry: false });
        assert_eq!((s.node(0, N_ACTIVE), s.node(0, N_STARTED), s.node(1, N_ACTIVE)), (1, 1, 0));
        assert_eq!((s.edge(0, E_ACTIVE), s.edge(0, E_CALLS)), (1, 1));
        s.apply(&DeltaKind::SpanEnd { node: 0, edge: Some(0), entry: true, error: true, duration_ns: -9 });
        s.apply(&DeltaKind::SpanEnd { node: 1, edge: None, entry: false, error: true, duration_ns: 50 });
        assert_eq!(s.node(0, N_ACTIVE), 0);
        assert_eq!((s.node(0, N_COMPLETED), s.node(0, N_ERRORS), s.node(0, N_LATENCY_SUM)), (1, 1, 0));
        assert_eq!((s.node(1, N_ANY_SPANS), s.node(1, N_COMPLETED), s.node(1, N_ERRORS)), (1, 0, 0));
        assert_eq!((s.edge(0, E_ACTIVE), s.edge(0, E_ERRORS)), (0, 1));
        s.apply(&DeltaKind::Event { node: None });
        s.apply(&DeltaKind::Event { node: Some(1) });
        s.apply(&DeltaKind::Log { node: 1, error: true });
        assert_eq!((s.node(1, N_EVENTS), s.node(1, N_LOGS), s.node(1, N_ERROR_LOGS)), (1, 1, 1));
    }

    #[test]
    fn latency_sums_wrap_but_differences_stay_exact() {
        let end = DeltaKind::SpanEnd { node: 0, edge: None, entry: true, error: false, duration_ns: i64::MAX };
        let mut past = StateVec::new(1, 0);
        past.apply(&end);
        past.apply(&end);
        let mut now = past.clone();
        now.apply(&end);
        assert_eq!(now.node(0, N_LATENCY_SUM).wrapping_sub(past.node(0, N_LATENCY_SUM)), i64::MAX);
    }
}
