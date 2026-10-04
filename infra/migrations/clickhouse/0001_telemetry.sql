-- karbird telemetry storage.
-- Deduplication: ReplacingMergeTree collapses rows with identical sorting keys
-- *eventually* (on merge). Queries that must not double count use FINAL.
-- This is NOT a guarantee that duplicates never exist on disk at any moment.
-- Retention: the TTLs below are only the initial values. `karbird-api bootstrap` sets them from
-- KARBIRD_{LOGS,SPANS,EVENTS}_RETENTION_DAYS (crates/karbird-storage/src/retention.rs).

CREATE TABLE IF NOT EXISTS spans
(
    tenant_id            LowCardinality(String),
    project_id           LowCardinality(String),
    environment_id       LowCardinality(String),
    service_id           LowCardinality(String),
    service_name         LowCardinality(String),
    service_namespace    LowCardinality(String),
    service_instance_id  String,
    service_version      LowCardinality(String),
    trace_id             String,
    span_id              String,
    parent_span_id       String,
    name                 String,
    kind                 UInt8,
    start_time           DateTime64(9, 'UTC'),
    end_time             DateTime64(9, 'UTC'),
    duration_ns          Int64,
    status_code          UInt8,
    status_message       String,
    target_entity_id     String,
    target_name          String,
    target_kind          LowCardinality(String),
    attributes           Map(LowCardinality(String), String),
    resource_attributes  Map(LowCardinality(String), String),
    batch_id             String,
    ingested_at          DateTime64(9, 'UTC'),
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 4
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMMDD(start_time)
ORDER BY (tenant_id, project_id, environment_id, service_id, start_time, trace_id, span_id)
TTL toDateTime(start_time) + INTERVAL 14 DAY;

CREATE TABLE IF NOT EXISTS logs
(
    tenant_id            LowCardinality(String),
    project_id           LowCardinality(String),
    environment_id       LowCardinality(String),
    service_id           LowCardinality(String),
    service_name         LowCardinality(String),
    service_instance_id  String,
    -- Deterministic per (batch_id, position): broker redelivery of the same
    -- batch collapses; an SDK retry (new batch_id) is NOT deduplicated because
    -- OTLP logs carry no unique ID and identical log lines can be legitimate.
    log_id               String,
    time                 DateTime64(9, 'UTC'),
    observed_time        DateTime64(9, 'UTC'),
    severity_number      UInt8,
    severity_text        LowCardinality(String),
    body                 String,
    trace_id             String,
    span_id              String,
    attributes           Map(LowCardinality(String), String),
    resource_attributes  Map(LowCardinality(String), String),
    batch_id             String,
    ingested_at          DateTime64(9, 'UTC'),
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 4,
    INDEX idx_body body TYPE tokenbf_v1(32768, 3, 0) GRANULARITY 4
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMMDD(time)
ORDER BY (tenant_id, project_id, environment_id, service_id, time, log_id)
TTL toDateTime(time) + INTERVAL 14 DAY;

CREATE TABLE IF NOT EXISTS events
(
    tenant_id            LowCardinality(String),
    project_id           LowCardinality(String),
    environment_id       LowCardinality(String),
    event_id             String,
    event_type           LowCardinality(String),
    service_id           LowCardinality(String),
    service_name         LowCardinality(String),
    service_instance_id  String,
    entity_id            String,
    event_time           DateTime64(9, 'UTC'),
    observed_time        DateTime64(9, 'UTC'),
    trace_id             String,
    span_id              String,
    source_id            String,
    source_epoch         String,
    source_sequence      Nullable(UInt64),
    attributes           Map(LowCardinality(String), String),
    payload_json         String,
    batch_id             String,
    ingested_at          DateTime64(9, 'UTC'),
    INDEX idx_trace_id trace_id TYPE bloom_filter(0.01) GRANULARITY 4
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMMDD(event_time)
ORDER BY (tenant_id, project_id, environment_id, event_time, event_id)
TTL toDateTime(event_time) + INTERVAL 30 DAY;
