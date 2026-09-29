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
/// `edge_index, t0, t1, t2, t3, flags`, times in milliseconds relative to the frame time.
pub const HOP_STRIDE: usize = 6;

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
        Self { edge, t, flags, trace }
    }

    /// A client span to a resource that is not instrumented, shifted by its service's skew.
    pub(crate) fn leaf(edge: u32, client: &SpanItem, trace: u64, shift: i64) -> Self {
        let mut flags = HOP_LEAF;
        if client.is_error {
            flags |= HOP_ERROR;
        }
        let (s, e) = (client.start_ns.saturating_add(shift), client.end_ns.saturating_add(shift));
        Self { edge, t: [s, s, e, e], flags, trace }
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
        let max_len = hops.iter().map(|h| h.t[3] - h.t[0]).max().unwrap_or(0);
        Self { hops, max_len }
    }

    /// Hops overlapping `[from, to]`, in departure order.
    pub(crate) fn overlapping(&self, from: i64, to: i64) -> impl Iterator<Item = &Hop> {
        let start = self.hops.partition_point(|h| h.t[0] < from.saturating_sub(self.max_len));
        self.hops[start..].iter().take_while(move |h| h.t[0] <= to).filter(move |h| h.t[3] >= from)
    }
}
