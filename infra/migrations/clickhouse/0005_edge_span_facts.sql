-- Narrow, replay-safe facts for edge traffic. The processor writes these before advancing its
-- PostgreSQL/Kafka offset; ReplacingMergeTree's stable span key makes retries idempotent.
-- Bootstrap applies the configured span retention to this table as well.
CREATE TABLE IF NOT EXISTS edge_span_facts
(
    tenant_id            LowCardinality(String),
    project_id           LowCardinality(String),
    environment_id       LowCardinality(String),
    service_id           LowCardinality(String),
    trace_id             String,
    span_id              String,
    parent_span_id       String,
    kind                 UInt8,
    target_entity_id     String,
    start_time           DateTime64(9, 'UTC'),
    request_size         Nullable(UInt64),
    response_size        Nullable(UInt64),
    ingested_at          DateTime64(9, 'UTC')
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMMDD(start_time)
ORDER BY (tenant_id, project_id, environment_id, service_id, start_time, trace_id, span_id)
TTL toDateTime(start_time) + INTERVAL 14 DAY;

-- Preserve historical edge charts when upgrading an existing database. This is a one-time
-- bootstrap backfill; subsequent rows are written idempotently by the processor.
INSERT INTO edge_span_facts
    (tenant_id, project_id, environment_id, service_id, trace_id, span_id, parent_span_id, kind,
     target_entity_id, start_time, request_size, response_size, ingested_at)
SELECT tenant_id, project_id, environment_id, service_id, trace_id, span_id, parent_span_id, kind,
       target_entity_id, start_time,
       toUInt64OrNull(attributes['http.request.body.size']),
       toUInt64OrNull(attributes['http.response.body.size']),
       ingested_at
FROM spans;
