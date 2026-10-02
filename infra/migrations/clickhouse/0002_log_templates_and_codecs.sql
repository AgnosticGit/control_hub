-- Log templates (ADR-0015): the processor stores the ID of each body's template, so lines of
-- one kind can be grouped and the archiver can sort them together.
ALTER TABLE logs ADD COLUMN IF NOT EXISTS template_id UInt64 DEFAULT 0 CODEC(ZSTD(1)) AFTER body;

-- Column codecs for the hot store. Only non-key columns: times that grow with the sort order
-- get delta encoding, text gets a stronger zstd level than the default LZ4. Applies to new
-- parts and to old ones as they merge.
ALTER TABLE logs MODIFY COLUMN observed_time DateTime64(9, 'UTC') CODEC(Delta, ZSTD(1));
ALTER TABLE logs MODIFY COLUMN ingested_at DateTime64(9, 'UTC') CODEC(Delta, ZSTD(1));
ALTER TABLE logs MODIFY COLUMN body String CODEC(ZSTD(3));
ALTER TABLE logs MODIFY COLUMN trace_id String CODEC(ZSTD(1));
ALTER TABLE logs MODIFY COLUMN span_id String CODEC(ZSTD(1));
ALTER TABLE logs MODIFY COLUMN batch_id String CODEC(ZSTD(1));
ALTER TABLE logs MODIFY COLUMN service_instance_id String CODEC(ZSTD(1));

ALTER TABLE spans MODIFY COLUMN end_time DateTime64(9, 'UTC') CODEC(Delta, ZSTD(1));
ALTER TABLE spans MODIFY COLUMN ingested_at DateTime64(9, 'UTC') CODEC(Delta, ZSTD(1));
ALTER TABLE spans MODIFY COLUMN duration_ns Int64 CODEC(T64, ZSTD(1));
ALTER TABLE spans MODIFY COLUMN name String CODEC(ZSTD(1));
ALTER TABLE spans MODIFY COLUMN status_message String CODEC(ZSTD(3));
ALTER TABLE spans MODIFY COLUMN parent_span_id String CODEC(ZSTD(1));
ALTER TABLE spans MODIFY COLUMN target_entity_id String CODEC(ZSTD(1));
ALTER TABLE spans MODIFY COLUMN target_name String CODEC(ZSTD(1));
ALTER TABLE spans MODIFY COLUMN batch_id String CODEC(ZSTD(1));
ALTER TABLE spans MODIFY COLUMN service_instance_id String CODEC(ZSTD(1));

ALTER TABLE events MODIFY COLUMN observed_time DateTime64(9, 'UTC') CODEC(Delta, ZSTD(1));
ALTER TABLE events MODIFY COLUMN ingested_at DateTime64(9, 'UTC') CODEC(Delta, ZSTD(1));
ALTER TABLE events MODIFY COLUMN payload_json String CODEC(ZSTD(3));
ALTER TABLE events MODIFY COLUMN batch_id String CODEC(ZSTD(1));
