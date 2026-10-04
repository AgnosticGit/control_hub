-- Machine and container metrics (ADR-0018): gauges and sums from the Karbird agent
-- (hostmetrics + docker_stats). One row per data point; `series_id` identifies the time series
-- (metric name + resource + point attributes), so a broker redelivery collapses on merge.
-- Retention: the TTL below is only the initial value; `karbird-api bootstrap` sets it from
-- KARBIRD_METRICS_RETENTION_DAYS (crates/karbird-storage/src/retention.rs).

CREATE TABLE IF NOT EXISTS metrics
(
    tenant_id            LowCardinality(String),
    project_id           LowCardinality(String),
    environment_id       LowCardinality(String),
    metric_name          LowCardinality(String),
    -- 1 gauge, 2 cumulative sum, 3 delta sum
    kind                 UInt8,
    unit                 LowCardinality(String),
    series_id            UInt64,
    -- `host.id`, or `host.name` when the agent does not report one
    host_id              LowCardinality(String),
    host_name            LowCardinality(String),
    container_id         String CODEC(ZSTD(1)),
    container_name       LowCardinality(String),
    container_hostname   LowCardinality(String),
    service_name         LowCardinality(String),
    attributes           Map(LowCardinality(String), String),
    value                Float64 CODEC(Gorilla, ZSTD(1)),
    time                 DateTime64(9, 'UTC') CODEC(Delta, ZSTD(1)),
    batch_id             String CODEC(ZSTD(1)),
    ingested_at          DateTime64(9, 'UTC') CODEC(Delta, ZSTD(1))
)
ENGINE = ReplacingMergeTree(ingested_at)
PARTITION BY toYYYYMMDD(time)
ORDER BY (tenant_id, project_id, environment_id, metric_name, series_id, time)
TTL toDateTime(time) + INTERVAL 14 DAY;
