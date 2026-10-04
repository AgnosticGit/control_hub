//! Display-time correction for clock skew between services.
//!
//! Every service stamps its spans with its own clock. Even on one host these clocks differ
//! by up to a millisecond (measured with Node on Windows), across hosts by more, so a
//! callee's server span can start before the caller's client span or end after it. As trace
//! viewers do, the engine shifts every span of a callee (its whole in-service subtree) so
//! that its entry span sits centred inside the caller's span when it does not already fit.
//! Shifts accumulate down the call chain. Stored data is never changed: the shift applies
//! only to what is drawn (request hops and timeline clips).

use std::collections::HashMap;

use crate::Engine;
use crate::types::SpanItem;

/// Bound on the parent walk, so broken or cyclic parent links cannot loop.
const MAX_WALK: usize = 64;

/// Memo of computed shifts for one pass over the spans, keyed by (trace id, span id).
pub(crate) type SkewCache<'a> = HashMap<(&'a str, &'a str), i64>;

/// Shift that puts `[start, end]` inside `[parent_start, parent_end]`; zero if it fits.
pub(crate) fn fit(start: i64, end: i64, parent_start: i64, parent_end: i64) -> i64 {
    if start >= parent_start && end <= parent_end {
        return 0;
    }
    if i128::from(end) - i128::from(start) <= i128::from(parent_end) - i128::from(parent_start) {
        let twice = i128::from(parent_start) + i128::from(parent_end) - i128::from(start) - i128::from(end);
        i64::try_from(twice / 2).unwrap_or(0)
    } else {
        // Longer than its caller (impossible without skew in the durations too): align starts.
        parent_start.saturating_sub(start)
    }
}

impl Engine {
    /// Display shift in nanoseconds for `span` (see the module docs).
    pub(crate) fn skew_ns<'a>(&'a self, span: &'a SpanItem, cache: &mut SkewCache<'a>) -> i64 {
        let key = (span.trace_id.as_str(), span.span_id.as_str());
        if let Some(&k) = cache.get(&key) {
            return k;
        }
        // Service boundaries between `span` and the first ancestor with a known shift (or the
        // root), innermost first: (entry span of the callee, caller's span).
        let mut boundaries: Vec<(&SpanItem, &SpanItem)> = Vec::new();
        let mut base = 0;
        let mut cur = span;
        for _ in 0..MAX_WALK {
            let Some(parent_id) = cur.parent_span_id.as_deref() else { break };
            let Some(parent) = self.spans.get(&(cur.trace_id.clone(), parent_id.to_owned())) else { break };
            if parent.service_id != cur.service_id {
                boundaries.push((cur, parent));
            }
            if let Some(&k) = cache.get(&(parent.trace_id.as_str(), parent.span_id.as_str())) {
                base = k;
                break;
            }
            cur = parent;
        }
        let shift = boundaries.iter().rev().fold(base, |caller_shift, (entry, caller)| {
            fit(
                entry.start_ns,
                entry.end_ns,
                caller.start_ns.saturating_add(caller_shift),
                caller.end_ns.saturating_add(caller_shift),
            )
        });
        cache.insert(key, shift);
        shift
    }
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn fitting_spans_are_left_alone() {
        assert_eq!(fit(2, 8, 0, 10), 0);
        assert_eq!(fit(0, 10, 0, 10), 0);
    }

    #[test]
    fn skewed_spans_are_centred_in_the_caller() {
        // Callee 2 units early: 1..8 inside 3..10 -> 3..10 (centred, both gaps 0).
        assert_eq!(fit(1, 8, 3, 10), 2);
        // Callee late: 6..10 inside 0..8 -> 2..6.
        assert_eq!(fit(6, 10, 0, 8), -4);
    }

    #[test]
    fn longer_callee_is_aligned_at_the_start() {
        assert_eq!(fit(5, 20, 0, 10), -5);
    }
}
