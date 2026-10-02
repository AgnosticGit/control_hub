-- Per-segment search index (ADR-0015): lets a reader skip segments that cannot match a filter
-- without fetching them. NULL means "not indexed" (segments archived before this migration, or
-- too many templates), and such segments are always opened.

ALTER TABLE archive_segments
    -- Template IDs (u64 stored bit-for-bit as BIGINT) of the segment's rows, sorted.
    ADD COLUMN template_ids BIGINT[],
    ADD COLUMN max_severity SMALLINT CHECK (max_severity BETWEEN 0 AND 255),
    -- Bloom filter over trace IDs; layout in controlhub-archive `index.rs`.
    ADD COLUMN trace_bloom BYTEA CHECK (octet_length(trace_bloom) BETWEEN 1 AND 131072);
