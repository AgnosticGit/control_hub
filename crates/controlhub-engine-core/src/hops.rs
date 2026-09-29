//! Individual request hops along edges, for animating what really happened.
//!
//! A hop carries four recorded timestamps instead of an aggregate:
//! `t0` request leaves the caller, `t1` it reaches the callee, `t2` the callee
//! answers, `t3` the answer is back at the caller. They come from the caller's
//! client span and the callee's server span. When the callee is not instrumented
//! (a database behind a client span), only `t0` and `t3` are known: the hop is a
//! "leaf" with `t1 = t0` and `t2 = t3`, and nothing is invented about how the
//! time splits between network and callee.

use crate::types::SpanItem;

/// Floats per hop in [`crate::VisibleState::hop_data`]:
/// `edge_index, t0, t1, t2, t3, flags, origin`, times in milliseconds relative to the frame
/// time. `origin` is the node index of the service where the hop's trace started (its root
/// span), or -1 when no ancestor with a known service is loaded.
pub const HOP_STRIDE: usize = 7;

/// `Hop::origin` when the trace's starting service is unknown.
pub(crate) const NO_ORIGIN: u32 = u32::MAX;

pub const HOP_ERROR: u32 = 1;
/// Callee not instrumented: only departure and return are known.
pub const HOP_LEAF: u32 = 2;
/// Part of the selected trace.
pub const HOP_HIGHLIGHT: u32 = 4;
/// Clock skew between services put the callee's span outside the caller's; the
/// callee's timestamps were shifted (see `skew.rs`) or clamped into order.
pub const HOP_CLAMPED: u32 = 8;
/// On an edge touching a muted service (see `Filters::muted_services`).
pub const HOP_DIMMED: u32 = 16;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hop {
    pub edge: u32,
    pub t: [i64; 4],
    pub flags: u32,
    pub trace: u64,
    /// Node index of the service where the trace started, or [`NO_ORIGIN`].
    pub origin: u32,
}

/// Clamps timestamps into non-decreasing order; returns whether anything moved.
fn monotonic(t: &mut [i64; 4]) -> bool {
    let mut moved = false;
    for i in 1..t.len() {
        if t[i] < t[i - 1] {
            t[i] = t[i - 1];
            moved = true;
        }
    }
    moved
}

impl Hop {
    /// A call between two instrumented services: the callee's server/consumer span and,
    /// when it is a client/producer span, the caller's parent span. `shift` holds the
    /// display clock-skew shifts of the caller's and the callee's spans.
    pub(crate) fn call(edge: u32, parent: &SpanItem, server: &SpanItem, trace: u64, shift: (i64, i64)) -> Self {
        let (ps, ss) = shift;
        let (s1, s2) = (server.start_ns.saturating_add(ss), server.end_ns.saturating_add(ss));
        let client = parent.kind == 3 || parent.kind == 4;
        let (t0, t3) =
            if client { (parent.start_ns.saturating_add(ps), parent.end_ns.saturating_add(ps)) } else { (s1, s2) };
        // A consumer answers nobody: the hop ends when processing ends.
        let t3 = if server.kind == 5 { s2 } else { t3 };
        let mut t = [t0, s1, s2, t3];
        let mut flags = 0;
        if monotonic(&mut t) || (client && ps != ss) {
            flags |= HOP_CLAMPED;
        }
        if server.is_error || (client && parent.is_error) {
            flags |= HOP_ERROR;
        }
        Self { edge, t, flags, trace, origin: NO_ORIGIN }
    }

    /// A client span to a resource that is not instrumented, shifted by its service's skew.
    pub(crate) fn leaf(edge: u32, client: &SpanItem, trace: u64, shift: i64) -> Self {
        let mut flags = HOP_LEAF;
        if client.is_error {
            flags |= HOP_ERROR;
        }
        let (s, e) = (client.start_ns.saturating_add(shift), client.end_ns.saturating_add(shift));
        Self { edge, t: [s, s, e, e], flags, trace, origin: NO_ORIGIN }
    }

    pub(crate) fn with_origin(mut self, origin: Option<u32>) -> Self {
        self.origin = origin.unwrap_or(NO_ORIGIN);
        self
    }
}

/// Hops sorted by departure time, for window queries.
#[derive(Debug, Default)]
pub(crate) struct HopIndex {
    hops: Vec<Hop>,
    max_len: i64,
}

impl HopIndex {
    pub(crate) fn new(mut hops: Vec<Hop>) -> Self {
        hops.sort_by(|a, b| a.t[0].cmp(&b.t[0]).then(a.trace.cmp(&b.trace)).then(a.edge.cmp(&b.edge)));
        let max_len = hops.iter().map(|h| h.t[3].saturating_sub(h.t[0])).max().unwrap_or(0);
        Self { hops, max_len }
    }

    /// Hops overlapping `[from, to]`, in departure order.
    pub(crate) fn overlapping(&self, from: i64, to: i64) -> impl Iterator<Item = &Hop> {
        let start = self.hops.partition_point(|h| h.t[0] < from.saturating_sub(self.max_len));
        self.hops[start..].iter().take_while(move |h| h.t[0] <= to).filter(move |h| h.t[3] >= from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn span(kind: u8, start: i64, end: i64, err: bool) -> SpanItem {
        SpanItem {
            trace_id: "t".into(),
            span_id: "s".into(),
            parent_span_id: None,
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
    fn client_parent_gives_four_distinct_times() {
        let h = Hop::call(3, &span(3, 0, 10, false), &span(2, 2, 8, false), 7, (0, 0));
        assert_eq!((h.edge, h.t, h.flags, h.trace), (3, [0, 2, 8, 10], 0, 7));
    }

    #[test]
    fn server_parent_uses_the_callee_span_only() {
        // An internal/server parent carries no departure time: the hop spans the callee only.
        let h = Hop::call(0, &span(2, 0, 100, true), &span(2, 20, 30, false), 0, (0, 0));
        assert_eq!(h.t, [20, 20, 30, 30]);
        assert_eq!(h.flags, 0, "a failing non-client parent does not mark the hop");
    }

    #[test]
    fn consumer_hop_ends_when_processing_ends() {
        let h = Hop::call(0, &span(4, 0, 5, false), &span(5, 10, 20, false), 0, (0, 0));
        assert_eq!(h.t, [0, 10, 20, 20]);
    }

    #[test]
    fn error_flags() {
        let ok = span(3, 0, 10, false);
        let bad = span(3, 0, 10, true);
        assert_eq!(Hop::call(0, &bad, &span(2, 1, 9, false), 0, (0, 0)).flags, HOP_ERROR);
        assert_eq!(Hop::call(0, &ok, &span(2, 1, 9, true), 0, (0, 0)).flags, HOP_ERROR);
        assert_eq!(Hop::leaf(0, &bad, 0, 0).flags, HOP_LEAF | HOP_ERROR);
        assert_eq!(Hop::leaf(0, &ok, 0, 0).flags, HOP_LEAF);
    }

    #[test]
    fn skew_shifts_are_applied_and_flagged() {
        let parent = span(3, 100, 200, false);
        let server = span(2, 0, 50, false);
        // Callee shifted by +125 lands at 125..175 inside the caller.
        let h = Hop::call(0, &parent, &server, 0, (0, 125));
        assert_eq!(h.t, [100, 125, 175, 200]);
        assert_eq!(h.flags, HOP_CLAMPED, "a shifted callee is marked");
        // Equal shifts on both sides mean nothing was corrected.
        let h = Hop::call(0, &span(3, 0, 100, false), &span(2, 10, 20, false), 0, (7, 7));
        assert_eq!((h.t, h.flags), ([7, 17, 27, 107], 0));
        let leaf = Hop::leaf(0, &span(3, 10, 20, false), 0, -4);
        assert_eq!(leaf.t, [6, 6, 16, 16]);
    }

    #[test]
    fn unshifted_skew_is_clamped_into_order() {
        let h = Hop::call(0, &span(3, 10, 20, false), &span(2, 0, 30, false), 0, (0, 0));
        assert_eq!(h.t, [10, 10, 30, 30]);
        assert_eq!(h.flags, HOP_CLAMPED);
    }

    #[test]
    fn extreme_times_saturate() {
        let h = Hop::call(
            0,
            &span(3, i64::MIN, i64::MAX, false),
            &span(2, i64::MAX, i64::MAX, false),
            0,
            (i64::MIN, i64::MAX),
        );
        assert!(h.t.windows(2).all(|w| w[0] <= w[1]));
        let leaf = Hop::leaf(0, &span(3, i64::MAX - 1, i64::MAX, false), 0, i64::MAX);
        assert_eq!(leaf.t, [i64::MAX; 4]);
        let index = HopIndex::new(vec![h, leaf]);
        assert_eq!(index.overlapping(i64::MIN, i64::MAX).count(), 2);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        /// Times are always non-decreasing, whatever the spans and shifts.
        #[test]
        fn hop_times_are_monotonic(
            p in (0u8..6, -1_000i64..1_000, 0i64..1_000, any::<bool>()),
            s in (0u8..6, -1_000i64..1_000, 0i64..1_000, any::<bool>()),
            shift in (-2_000i64..2_000, -2_000i64..2_000),
        ) {
            let parent = span(p.0, p.1, p.1 + p.2, p.3);
            let server = span(s.0, s.1, s.1 + s.2, s.3);
            let h = Hop::call(0, &parent, &server, 0, shift);
            prop_assert!(h.t.windows(2).all(|w| w[0] <= w[1]), "{:?}", h.t);
            prop_assert_eq!(h.flags & HOP_ERROR != 0, server.is_error || ((p.0 == 3 || p.0 == 4) && parent.is_error));
            prop_assert_eq!(h.flags & (HOP_LEAF | HOP_HIGHLIGHT | HOP_DIMMED), 0);
        }

        /// The index returns exactly the hops overlapping the closed window, in departure order.
        #[test]
        fn overlapping_matches_a_linear_scan(
            raw in prop::collection::vec((-500i64..500, 0i64..300, 0u32..4), 0..60),
            from in -700i64..700,
            len in 0i64..400,
        ) {
            let hops: Vec<Hop> = raw.iter().enumerate()
                .map(|(i, &(s, d, e))| Hop { edge: e, t: [s, s, s + d, s + d], flags: 0, trace: i as u64, origin: NO_ORIGIN })
                .collect();
            let to = from + len;
            let index = HopIndex::new(hops.clone());
            let got: Vec<u64> = index.overlapping(from, to).map(|h| h.trace).collect();
            let mut want: Vec<&Hop> = hops.iter().filter(|h| h.t[0] <= to && h.t[3] >= from).collect();
            want.sort_by(|a, b| a.t[0].cmp(&b.t[0]).then(a.trace.cmp(&b.trace)).then(a.edge.cmp(&b.edge)));
            prop_assert_eq!(got, want.iter().map(|h| h.trace).collect::<Vec<_>>());
        }
    }
}
