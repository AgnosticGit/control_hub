//! CPU-only helpers for bounded broker batch preparation.

pub const MAX_DEAD_LETTER_PAYLOAD: usize = 1024 * 1024;

/// Drain only records already ready in the consumer; this batches storage work without adding
/// a timer delay. The single record that crosses the byte budget is retained, never discarded.
pub fn drain_ready_batch<T>(
    first: T,
    max_records: usize,
    max_bytes: usize,
    mut size_of: impl FnMut(&T) -> usize,
    mut next_ready: impl FnMut() -> Option<T>,
) -> Vec<T> {
    let mut bytes = size_of(&first);
    let mut batch = Vec::with_capacity(max_records.max(1));
    batch.push(first);
    while batch.len() < max_records.max(1) && bytes < max_bytes {
        let Some(next) = next_ready() else { break };
        bytes = bytes.saturating_add(size_of(&next));
        batch.push(next);
    }
    batch
}

/// Dead-letter rows keep the raw record only up to the configured payload bound.
pub fn dead_letter_payload(payload: &[u8]) -> Option<&[u8]> {
    (payload.len() <= MAX_DEAD_LETTER_PAYLOAD).then_some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn drains_ready_items_without_dropping_the_item_that_crosses_byte_limit() {
        let mut queued = VecDeque::from([2usize, 4, 1]);
        assert_eq!(drain_ready_batch(3, 8, 6, |size| *size, || queued.pop_front()), [3, 2, 4]);
    }

    #[test]
    fn respects_record_limit_and_never_uses_zero_capacity() {
        let mut queued = VecDeque::from([2, 3]);
        assert_eq!(drain_ready_batch(0, 0, 16, |_| 1, || queued.pop_front()), [0]);
    }

    #[test]
    fn dead_letter_limit_is_inclusive() {
        assert_eq!(dead_letter_payload(b"abc"), Some(&b"abc"[..]));
        assert!(dead_letter_payload(&vec![0; MAX_DEAD_LETTER_PAYLOAD + 1]).is_none());
    }
}
