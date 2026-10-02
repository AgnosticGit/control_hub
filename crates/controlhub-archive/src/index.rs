//! Per-segment search index kept in the catalog, so a reader can skip segments that cannot hold
//! a match without fetching them: the segment's template IDs, its highest severity and a bloom
//! filter over its trace IDs.
//!
//! The bloom filter is checked inside PostgreSQL (`get_bit`), so its layout is part of the
//! catalog contract: bit `p` is bit `p % 8` (least significant first) of byte `p / 8`, and the
//! [`BLOOM_HASHES`] probe positions of a trace ID are `(h1 + i * h2) % bits` with `(h1, h2)` from
//! [`bloom_probe`].

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::LogRecord;
use crate::template::{parse, template_id};

/// Probes per trace ID.
pub const BLOOM_HASHES: u32 = 7;
/// About 1 % false positives at 7 probes.
const BLOOM_BITS_PER_ITEM: usize = 10;
const BLOOM_MIN_BITS: usize = 64;
/// 128 KiB. More distinct traces than this holds only raise the false-positive rate.
pub const BLOOM_MAX_BITS: usize = 1 << 20;
/// Segments with more distinct templates are not template-indexed (always opened).
pub const MAX_INDEXED_TEMPLATES: usize = 4096;

/// The two hashes a trace ID's probe positions derive from; both fit 32 bits, `h2` is odd.
pub fn bloom_probe(trace_id: &str) -> (u32, u32) {
    let d = Sha256::digest(trace_id.as_bytes());
    let h1 = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
    let h2 = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) | 1;
    (h1, h2)
}

fn positions(trace_id: &str, bits: usize) -> impl Iterator<Item = usize> {
    let (h1, h2) = bloom_probe(trace_id);
    let bits = bits as u64;
    (0..u64::from(BLOOM_HASHES)).map(move |i| ((u64::from(h1) + i * u64::from(h2)) % bits) as usize)
}

/// Bloom filter bytes over `trace_ids` (duplicates allowed). Never empty.
pub fn bloom_build<'a>(trace_ids: impl IntoIterator<Item = &'a str>) -> Vec<u8> {
    let distinct: BTreeSet<&str> = trace_ids.into_iter().filter(|t| !t.is_empty()).collect();
    let bits = (distinct.len() * BLOOM_BITS_PER_ITEM).clamp(BLOOM_MIN_BITS, BLOOM_MAX_BITS).next_multiple_of(8);
    let mut out = vec![0u8; bits / 8];
    for t in distinct {
        for p in positions(t, bits) {
            out[p / 8] |= 1 << (p % 8);
        }
    }
    out
}

/// Whether the filter may hold `trace_id`. `false` is definite; an empty filter holds nothing.
pub fn bloom_may_contain(bloom: &[u8], trace_id: &str) -> bool {
    if bloom.is_empty() {
        return false;
    }
    positions(trace_id, bloom.len() * 8).all(|p| bloom[p / 8] & (1 << (p % 8)) != 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentIndex {
    /// Sorted, unique. `None` when the segment has more than [`MAX_INDEXED_TEMPLATES`].
    pub template_ids: Option<Vec<u64>>,
    pub max_severity: u8,
    pub trace_bloom: Vec<u8>,
}

/// Index of a segment's rows. Template IDs are those the reader reports for each row.
pub fn segment_index(records: &[LogRecord]) -> SegmentIndex {
    let ids: BTreeSet<u64> = records.iter().map(|r| template_id(&parse(&r.body).template)).collect();
    SegmentIndex {
        template_ids: (ids.len() <= MAX_INDEXED_TEMPLATES).then(|| ids.into_iter().collect()),
        max_severity: records.iter().map(|r| r.severity_number).max().unwrap_or(0),
        trace_bloom: bloom_build(records.iter().map(|r| r.trace_id.as_str())),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    fn trace(i: u32) -> String {
        format!("{i:032x}")
    }

    #[test]
    fn bloom_sizes_are_bounded_and_whole_bytes() {
        assert_eq!(bloom_build([]).len(), BLOOM_MIN_BITS / 8, "an empty segment still has a filter");
        let many: Vec<String> = (0..200_000).map(trace).collect();
        assert_eq!(bloom_build(many.iter().map(String::as_str)).len(), BLOOM_MAX_BITS / 8);
        let some: Vec<String> = (0..100).map(trace).collect();
        assert_eq!(bloom_build(some.iter().map(String::as_str)).len(), 1000 / 8);
    }

    #[test]
    fn empty_trace_ids_are_not_indexed() {
        let b = bloom_build(["", ""]);
        assert!(b.iter().all(|&x| x == 0));
        assert!(!bloom_may_contain(&b, &trace(1)));
        assert!(!bloom_may_contain(&[], &trace(1)));
    }

    #[test]
    fn bit_layout_is_least_significant_first() {
        // The layout PostgreSQL's get_bit reads: bit p is bit p % 8 of byte p / 8.
        let b = bloom_build([trace(7).as_str()]);
        let bits = b.len() * 8;
        let (h1, h2) = bloom_probe(&trace(7));
        for i in 0..u64::from(BLOOM_HASHES) {
            let p = ((u64::from(h1) + i * u64::from(h2)) % bits as u64) as usize;
            assert_ne!(b[p / 8] & (1 << (p % 8)), 0, "probe {i} at bit {p}");
        }
        assert!(b.iter().map(|x| x.count_ones()).sum::<u32>() <= BLOOM_HASHES);
    }

    #[test]
    fn false_positive_rate_is_low() {
        let ids: Vec<String> = (0..1_000).map(trace).collect();
        let b = bloom_build(ids.iter().map(String::as_str));
        let fp = (1_000_000..1_010_000).filter(|&i| bloom_may_contain(&b, &trace(i))).count();
        assert!(fp < 300, "{fp} false positives in 10000");
    }

    #[test]
    fn segment_index_matches_what_the_reader_reports() {
        let rec = |body: &str, sev: u8, t: &str| LogRecord {
            body: body.into(),
            severity_number: sev,
            trace_id: t.into(),
            ..LogRecord::default()
        };
        let records = [
            rec("order 1 created", 9, ""),
            rec("order 2 created", 13, &trace(1)),
            rec("payment failed", 17, &trace(2)),
        ];
        let idx = segment_index(&records);
        let want: BTreeSet<u64> = records.iter().map(|r| template_id(&parse(&r.body).template)).collect();
        assert_eq!(idx.template_ids.as_deref().unwrap(), want.into_iter().collect::<Vec<_>>());
        assert_eq!(idx.template_ids.as_ref().unwrap().len(), 2, "one template per kind of line");
        assert_eq!(idx.max_severity, 17);
        assert!(bloom_may_contain(&idx.trace_bloom, &trace(1)) && bloom_may_contain(&idx.trace_bloom, &trace(2)));

        // Letters only: digits would make the word a variable and every line one template.
        let word = |mut i: usize| {
            let mut w = String::new();
            loop {
                w.push(char::from(b'a' + (i % 26) as u8));
                i /= 26;
                if i == 0 {
                    return w;
                }
            }
        };
        let wide: Vec<LogRecord> =
            (0..=MAX_INDEXED_TEMPLATES).map(|i| rec(&format!("kind {} happened", word(i)), 9, "")).collect();
        assert_eq!(segment_index(&wide).template_ids, None, "too many templates: not indexed");
        assert_eq!(segment_index(&[]).max_severity, 0);
    }

    proptest! {
        /// No false negatives, ever: a segment holding a trace is never skipped.
        #[test]
        fn bloom_has_no_false_negatives(ids in proptest::collection::vec("[0-9a-f]{32}", 0..300)) {
            let b = bloom_build(ids.iter().map(String::as_str));
            for id in &ids {
                prop_assert!(bloom_may_contain(&b, id));
            }
        }
    }
}
