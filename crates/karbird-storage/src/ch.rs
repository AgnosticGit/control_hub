//! ClickHouse: telemetry rows, batched inserts and scoped analytical queries.
//! Every query binds tenant/project/environment; there is no raw-SQL entry point.

use clickhouse::{Client, Row};
use serde::{Deserialize, Serialize};

use crate::retention::{self, Table};
use crate::{Result, Scope, StorageError};

#[derive(Debug, Clone)]
pub struct ChConfig {
    pub url: String,
    pub database: String,
    pub user: String,
    pub password: String,
}

#[derive(Clone)]
pub struct ChStore {
    client: Client,
}

impl std::fmt::Debug for ChStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChStore").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------- rows

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct SpanRow {
    pub tenant_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub service_id: String,
    pub service_name: String,
    pub service_namespace: String,
    pub service_instance_id: String,
    pub service_version: String,
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    pub kind: u8,
    pub start_time: i64,
    pub end_time: i64,
    pub duration_ns: i64,
    pub status_code: u8,
    pub status_message: String,
    pub target_entity_id: String,
    pub target_name: String,
    pub target_kind: String,
    pub attributes: Vec<(String, String)>,
    pub resource_attributes: Vec<(String, String)>,
    pub batch_id: String,
    pub ingested_at: i64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct LogRow {
    pub tenant_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub service_id: String,
    pub service_name: String,
    pub service_instance_id: String,
    pub log_id: String,
    pub time: i64,
    pub observed_time: i64,
    pub severity_number: u8,
    pub severity_text: String,
    pub body: String,
    /// ID of the body's template (`karbird_archive::template_id`); lines of one kind share it.
    pub template_id: u64,
    pub trace_id: String,
    pub span_id: String,
    pub attributes: Vec<(String, String)>,
    pub resource_attributes: Vec<(String, String)>,
    pub batch_id: String,
    pub ingested_at: i64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct EventRow {
    pub tenant_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub event_id: String,
    pub event_type: String,
    pub service_id: String,
    pub service_name: String,
    pub service_instance_id: String,
    pub entity_id: String,
    pub event_time: i64,
    pub observed_time: i64,
    pub trace_id: String,
    pub span_id: String,
    pub source_id: String,
    pub source_epoch: String,
    pub source_sequence: Option<u64>,
    pub attributes: Vec<(String, String)>,
    pub payload_json: String,
    pub batch_id: String,
    pub ingested_at: i64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct MetricRow {
    pub tenant_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub metric_name: String,
    pub kind: u8,
    pub unit: String,
    pub series_id: u64,
    pub host_id: String,
    pub host_name: String,
    pub container_id: String,
    pub container_name: String,
    pub container_hostname: String,
    /// `container.image.name`, e.g. `postgres:16-alpine`.
    pub container_image: String,
    /// Docker Compose service name (label `com.docker.compose.service`), reported by the agent as
    /// `docker.compose.service`.
    pub container_service: String,
    pub service_name: String,
    pub attributes: Vec<(String, String)>,
    pub value: f64,
    pub time: i64,
    pub batch_id: String,
    pub ingested_at: i64,
}

pub const METRIC_KIND_GAUGE: u8 = 1;
pub const METRIC_KIND_CUMULATIVE_SUM: u8 = 2;
pub const METRIC_KIND_DELTA_SUM: u8 = 3;

/// The machine and container metrics the infrastructure view reads (hostmetrics + docker_stats).
pub const INFRA_METRIC_NAMES: &[&str] = &[
    "system.cpu.time",
    "system.cpu.utilization",
    "system.cpu.logical.count",
    "system.memory.usage",
    "system.memory.limit",
    "system.filesystem.usage",
    "system.network.io",
    "container.cpu.usage.total",
    "container.cpu.utilization",
    "container.memory.usage.total",
    "container.memory.usage.limit",
    "container.network.io.usage.rx_bytes",
    "container.network.io.usage.tx_bytes",
];

// ---------------------------------------------------------------- query results

/// First and last point of one series in a window.
#[derive(Debug, Clone, PartialEq, Row, Serialize, Deserialize)]
pub struct InfraSeriesRow {
    pub host_id: String,
    pub host_name: String,
    pub container_id: String,
    pub container_name: String,
    pub container_hostname: String,
    pub container_image: String,
    pub container_service: String,
    pub service_name: String,
    pub metric_name: String,
    pub series_id: u64,
    pub attributes: Vec<(String, String)>,
    pub first_value: f64,
    pub first_time: i64,
    pub last_value: f64,
    pub last_time: i64,
}

/// Metrics the infrastructure timeline reads (a subset of [`INFRA_METRIC_NAMES`]).
pub const INFRA_TIMELINE_METRIC_NAMES: &[&str] = &[
    "system.cpu.time",
    "system.cpu.utilization",
    "system.memory.usage",
    "system.memory.limit",
    "container.cpu.usage.total",
    "container.memory.usage.total",
    "container.memory.usage.limit",
];

/// One class of series of a machine or container in one time bucket.
#[derive(Debug, Clone, PartialEq, Row, Serialize, Deserialize)]
pub struct InfraBucketRow {
    pub host_id: String,
    pub host_name: String,
    pub container_id: String,
    pub container_name: String,
    pub service_name: String,
    pub metric_name: String,
    /// `busy` / `idle` for CPU time and utilization, `used` / `other` for memory usage, else empty.
    pub class: String,
    /// Bucket index counted from the query's `base`.
    pub bucket: i64,
    /// Series summed into this row.
    pub series: u64,
    /// Sums over the series of their first, last and highest value in the bucket.
    pub first_value: f64,
    pub last_value: f64,
    pub max_value: f64,
    pub first_time: i64,
    pub last_time: i64,
}

/// Where one service instance connects to a database or queue: the address and port of its
/// client spans (`server.address`, else `net.peer.name`) and where the caller runs.
#[derive(Debug, Clone, PartialEq, Row, Serialize, Deserialize)]
pub struct ResourceAddressRow {
    pub target_entity_id: String,
    pub target_name: String,
    pub target_kind: String,
    pub address: String,
    pub port: String,
    pub caller_service_id: String,
    pub caller_host_name: String,
    pub caller_host_id: String,
    pub caller_container_id: String,
    pub calls: u64,
}

/// Request latency of one service instance now and in the baseline before it.
#[derive(Debug, Clone, PartialEq, Row, Serialize, Deserialize)]
pub struct InstanceLatencyRow {
    pub service_id: String,
    pub service_name: String,
    pub service_instance_id: String,
    pub host_name: String,
    pub host_id: String,
    pub container_id: String,
    pub requests: u64,
    pub p95_ns: f64,
    pub baseline_requests: u64,
    pub baseline_p95_ns: f64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct ServiceSummaryRow {
    pub service_id: String,
    pub service_name: String,
    pub requests: u64,
    pub errors: u64,
    pub p95_ns: f64,
    pub last_seen: i64,
    pub instances: u64,
    pub version: String,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct LogServiceRow {
    pub service_id: String,
    pub service_name: String,
    pub logs: u64,
    pub error_logs: u64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct CallEdgeRow {
    pub source_service_id: String,
    pub target_service_id: String,
    pub calls: u64,
    pub errors: u64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct ResourceEdgeRow {
    pub service_id: String,
    pub target_entity_id: String,
    pub target_name: String,
    pub target_kind: String,
    pub calls: u64,
    pub errors: u64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct ExternalClientEdgeRow {
    pub service_id: String,
    pub calls: u64,
    pub errors: u64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct TraceSummaryRow {
    pub trace_id: String,
    pub trace_start: i64,
    pub trace_end: i64,
    pub span_count: u64,
    pub error_count: u64,
    pub root_name: String,
    pub root_service: String,
    pub services: Vec<String>,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct TimelineSpanRow {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub service_id: String,
    pub service_name: String,
    pub target_entity_id: String,
    pub name: String,
    pub kind: u8,
    pub start_time: i64,
    pub end_time: i64,
    pub status_code: u8,
    pub ingested_at: i64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
pub struct TimelineLogRow {
    pub log_id: String,
    pub service_id: String,
    pub service_name: String,
    pub time: i64,
    pub severity_number: u8,
    pub trace_id: String,
    pub ingested_at: i64,
}

/// Logs of one service in one archive window.
#[derive(Debug, Clone, PartialEq, Eq, Row, Serialize, Deserialize)]
pub struct LogGroupRow {
    pub tenant_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub service_id: String,
    pub rows: u64,
}

#[derive(Debug, Clone, Row, Serialize, Deserialize)]
struct EarliestRow {
    earliest: i64,
    rows: u64,
}

#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub from_ns: i64,
    pub to_ns: i64,
    pub service_id: Option<String>,
    pub min_severity: Option<u8>,
    pub trace_id: Option<String>,
    pub text: Option<String>,
    /// Lines of one kind (see `karbird_archive::template_id`).
    pub template_id: Option<u64>,
    /// Exclusive cursor `(time_ns, log_id)`; results are newest-first.
    pub before: Option<(i64, String)>,
    pub limit: u32,
}

#[derive(Debug, Clone, Default)]
pub struct TraceQuery {
    pub from_ns: i64,
    pub to_ns: i64,
    pub service_id: Option<String>,
    pub only_errors: bool,
    /// Exclusive cursor `(start_ns, trace_id)`; newest-first.
    pub before: Option<(i64, String)>,
    pub limit: u32,
}

const SCOPE_WHERE: &str = "tenant_id = ? AND project_id = ? AND environment_id = ?";
/// How far before a window parent/overlapping spans may start (5 min).
const SPAN_LOOKBACK_NS: i64 = 300_000_000_000;

fn traces_sql(tq: &TraceQuery) -> String {
    let mut sql = format!(
        "SELECT trace_id, min(start_time) AS trace_start, max(end_time) AS trace_end, count() AS span_count,
                countIf(status_code = 2) AS error_count,
                argMin(name, start_time) AS root_name, argMin(service_name, start_time) AS root_service,
                arraySort(groupUniqArray(toString(service_name))) AS services
         FROM spans FINAL
         WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)"
    );
    if tq.service_id.is_some() {
        sql.push_str(" AND trace_id IN (SELECT trace_id FROM spans WHERE tenant_id = ? AND project_id = ? AND environment_id = ? AND service_id = ? AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?))");
    }
    sql.push_str(" GROUP BY trace_id");
    let mut having = Vec::new();
    if tq.only_errors {
        having.push("error_count > 0");
    }
    if tq.before.is_some() {
        having.push("(trace_start, trace_id) < (fromUnixTimestamp64Nano(?), ?)");
    }
    if !having.is_empty() {
        sql.push_str(" HAVING ");
        sql.push_str(&having.join(" AND "));
    }
    sql.push_str(" ORDER BY trace_start DESC, trace_id DESC LIMIT ?");
    sql
}

fn logs_sql(lq: &LogQuery) -> String {
    let mut sql = format!(
        "SELECT ?fields FROM logs FINAL
         WHERE {SCOPE_WHERE} AND time >= fromUnixTimestamp64Nano(?) AND time < fromUnixTimestamp64Nano(?)"
    );
    if lq.service_id.is_some() {
        sql.push_str(" AND service_id = ?");
    }
    if lq.min_severity.is_some() {
        sql.push_str(" AND severity_number >= ?");
    }
    if lq.trace_id.is_some() {
        sql.push_str(" AND trace_id = ?");
    }
    if lq.text.is_some() {
        sql.push_str(" AND positionCaseInsensitive(body, ?) > 0");
    }
    if lq.template_id.is_some() {
        sql.push_str(" AND template_id = ?");
    }
    if lq.before.is_some() {
        sql.push_str(" AND (time, log_id) < (fromUnixTimestamp64Nano(?), ?)");
    }
    sql.push_str(" ORDER BY time DESC, log_id DESC LIMIT ?");
    sql
}

fn events_sql(with_trace: bool) -> String {
    let mut sql = format!(
        "SELECT ?fields FROM events FINAL
         WHERE {SCOPE_WHERE} AND event_time >= fromUnixTimestamp64Nano(?) AND event_time < fromUnixTimestamp64Nano(?)"
    );
    if with_trace {
        sql.push_str(" AND trace_id = ?");
    }
    sql.push_str(" ORDER BY event_time DESC, event_id DESC LIMIT ?");
    sql
}

/// Archive reads span all tenants (the archiver's job) unless narrowed to one scope.
fn log_groups_sql(scoped: bool) -> String {
    let scope = if scoped { format!(" AND {SCOPE_WHERE}") } else { String::new() };
    format!(
        "SELECT tenant_id, project_id, environment_id, service_id, count() AS rows FROM logs FINAL
         WHERE time >= fromUnixTimestamp64Nano(?) AND time < fromUnixTimestamp64Nano(?){scope}
         GROUP BY tenant_id, project_id, environment_id, service_id
         ORDER BY tenant_id, project_id, environment_id, service_id"
    )
}

fn count_logs_sql(scoped: bool) -> String {
    let scope = if scoped { format!(" AND {SCOPE_WHERE}") } else { String::new() };
    format!(
        "SELECT count() FROM logs FINAL
         WHERE time >= fromUnixTimestamp64Nano(?) AND time < fromUnixTimestamp64Nano(?){scope}"
    )
}

fn infra_series_sql() -> String {
    let names = INFRA_METRIC_NAMES.iter().map(|n| format!("'{n}'")).collect::<Vec<_>>().join(", ");
    format!(
        "SELECT host_id, any(host_name) AS host_name, container_id, any(container_name) AS container_name,
                any(container_hostname) AS container_hostname, any(container_image) AS container_image,
                any(container_service) AS container_service, any(service_name) AS service_name,
                metric_name, series_id, any(attributes) AS attributes,
                argMin(value, time) AS first_value, min(time) AS first_time,
                argMax(value, time) AS last_value, max(time) AS last_time
         FROM metrics FINAL
         WHERE {SCOPE_WHERE} AND metric_name IN ({names})
           AND time >= fromUnixTimestamp64Nano(?) AND time <= fromUnixTimestamp64Nano(?)
         GROUP BY host_id, container_id, metric_name, series_id
         ORDER BY host_id, container_id, metric_name, series_id
         LIMIT ?"
    )
}

/// Per machine/container, metric, class and bucket of `bucket_ns` counted from `base`: sums of
/// the first, last and highest value of each series in the bucket.
fn infra_buckets_sql() -> String {
    let names = INFRA_TIMELINE_METRIC_NAMES.iter().map(|n| format!("'{n}'")).collect::<Vec<_>>().join(", ");
    format!(
        "SELECT host_id, any(host_name) AS host_name, container_id, any(container_name) AS container_name,
                any(service_name) AS service_name, metric_name, class, bucket, count() AS series,
                sum(first_v) AS first_value, sum(last_v) AS last_value, sum(max_v) AS max_value,
                min(first_t) AS first_time, max(last_t) AS last_time
         FROM (
             SELECT host_id, any(host_name) AS host_name, container_id, any(container_name) AS container_name,
                    any(service_name) AS service_name, metric_name, series_id,
                    multiIf(metric_name IN ('system.cpu.time', 'system.cpu.utilization'),
                                if(attributes['state'] IN ('idle', 'wait'), 'idle', 'busy'),
                            metric_name = 'system.memory.usage', if(attributes['state'] = 'used', 'used', 'other'),
                            '') AS class,
                    intDiv(toUnixTimestamp64Nano(time) - ?, ?) AS bucket,
                    argMin(value, time) AS first_v, argMax(value, time) AS last_v, max(value) AS max_v,
                    min(time) AS first_t, max(time) AS last_t
             FROM metrics FINAL
             WHERE {SCOPE_WHERE} AND metric_name IN ({names})
               AND time >= fromUnixTimestamp64Nano(?) AND time < fromUnixTimestamp64Nano(?)
             GROUP BY host_id, container_id, metric_name, series_id, class, bucket
         )
         GROUP BY host_id, container_id, metric_name, class, bucket
         ORDER BY host_id, container_id, metric_name, class, bucket
         LIMIT ?"
    )
}

/// Server/consumer and root spans per instance; `p95` covers `[now_from, to)`, the baseline
/// `[from, now_from)`.
fn instance_latency_sql() -> String {
    format!(
        "SELECT service_id, any(service_name) AS service_name, service_instance_id,
                any(resource_attributes['host.name']) AS host_name,
                any(resource_attributes['host.id']) AS host_id,
                any(resource_attributes['container.id']) AS container_id,
                countIf(start_time >= fromUnixTimestamp64Nano(?)) AS requests,
                quantileIf(0.95)(toFloat64(duration_ns), start_time >= fromUnixTimestamp64Nano(?)) AS p95_ns,
                countIf(start_time < fromUnixTimestamp64Nano(?)) AS baseline_requests,
                quantileIf(0.95)(toFloat64(duration_ns), start_time < fromUnixTimestamp64Nano(?)) AS baseline_p95_ns,
                max(end_time) AS last_seen
         FROM spans FINAL
         WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
           AND (kind IN (2, 5) OR parent_span_id = '')
         GROUP BY service_id, service_instance_id
         ORDER BY service_id, service_instance_id
         LIMIT ?"
    )
}

/// Client/producer spans to databases and queues in `[from, to)`, per target, address and caller.
fn resource_addresses_sql() -> String {
    format!(
        "SELECT target_entity_id, target_name, target_kind,
                if(attributes['server.address'] != '', attributes['server.address'], attributes['net.peer.name']) AS address,
                if(attributes['server.port'] != '', attributes['server.port'], attributes['net.peer.port']) AS port,
                service_id AS caller_service_id,
                resource_attributes['host.name'] AS caller_host_name,
                resource_attributes['host.id'] AS caller_host_id,
                resource_attributes['container.id'] AS caller_container_id,
                count() AS calls
         FROM spans FINAL
         WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
           AND kind IN (3, 4) AND target_kind IN ('database', 'queue') AND target_entity_id != ''
         GROUP BY target_entity_id, target_name, target_kind, address, port, caller_service_id, caller_host_name, caller_host_id, caller_container_id
         ORDER BY target_entity_id, address, port, caller_service_id, caller_host_name, caller_host_id, caller_container_id
         LIMIT ?"
    )
}

fn bind_scope(q: clickhouse::query::Query, s: &Scope) -> clickhouse::query::Query {
    q.bind(s.tenant_id.as_str()).bind(s.project_id.as_str()).bind(s.environment_id.as_str())
}

impl ChStore {
    pub fn new(cfg: &ChConfig) -> Self {
        let client = Client::default()
            .with_url(&cfg.url)
            .with_database(&cfg.database)
            .with_user(&cfg.user)
            .with_password(&cfg.password)
            // Async inserts are not used: an insert returns only after the data is written.
            .with_setting("wait_end_of_query", "1");
        Self { client }
    }

    pub async fn ping(&self) -> Result<()> {
        self.client.query("SELECT 1").execute().await?;
        Ok(())
    }

    /// Apply `infra/migrations/clickhouse/*.sql` in order, tracking applied files.
    pub async fn migrate(&self, migrations: &[(&str, &str)]) -> Result<Vec<String>> {
        self.client
            .query(
                "CREATE TABLE IF NOT EXISTS schema_migrations (name String, applied_at DateTime64(3, 'UTC') DEFAULT now64(3))
                 ENGINE = MergeTree ORDER BY name",
            )
            .execute()
            .await?;
        let applied: Vec<String> =
            self.client.query("SELECT name FROM schema_migrations").fetch_all::<String>().await?;
        let mut done = Vec::new();
        for (name, sql) in migrations {
            if applied.iter().any(|a| a == name) {
                continue;
            }
            for stmt in split_statements(sql) {
                self.client.query(&stmt).execute().await?;
            }
            self.client.query("INSERT INTO schema_migrations (name) VALUES (?)").bind(*name).execute().await?;
            done.push((*name).to_owned());
        }
        Ok(done)
    }

    /// Create/update a least-privilege ClickHouse user (bootstrap only).
    pub async fn ensure_user(&self, user: &str, password: &str, grants: &[&str]) -> Result<()> {
        let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !ok(user) || !ok(password) {
            return Err(StorageError::Other("invalid user name or password charset".into()));
        }
        self.client
            .query(&format!("CREATE USER IF NOT EXISTS {user} IDENTIFIED WITH sha256_password BY '{password}'"))
            .execute()
            .await?;
        self.client
            .query(&format!("ALTER USER {user} IDENTIFIED WITH sha256_password BY '{password}'"))
            .execute()
            .await?;
        for g in grants {
            self.client.query(&format!("GRANT {g} TO {user}")).execute().await?;
        }
        Ok(())
    }

    // ------------------------------------------------------------ retention

    /// The table's TTL in days; `None` if it is not a plain day TTL on the table's time column.
    /// Read-only users see the tables they may select from, so every service can check this.
    pub async fn ttl_days(&self, table: Table) -> Result<Option<u32>> {
        let engines: Vec<String> = self
            .client
            .query("SELECT engine_full FROM system.tables WHERE database = currentDatabase() AND name = ?")
            .bind(table.name())
            .fetch_all()
            .await?;
        let engine = engines
            .first()
            .ok_or_else(|| StorageError::Other(format!("ClickHouse table `{}` not found", table.name())))?;
        Ok(retention::parse_ttl_days(engine, table.time_column()))
    }

    /// Fails with an operator-facing message when `configured` differs from the table's TTL.
    pub async fn verify_retention(&self, table: Table, configured: u32) -> Result<()> {
        retention::check(table, configured, self.ttl_days(table).await?).map_err(StorageError::Other)
    }

    /// Rows a TTL of `days` would delete (bootstrap only, before shortening retention).
    pub async fn rows_older_than(&self, table: Table, days: u32) -> Result<u64> {
        let sql =
            format!("SELECT count() FROM {} WHERE {} < now() - INTERVAL ? DAY", table.name(), table.time_column());
        Ok(self.client.query(&sql).bind(days).fetch_one::<u64>().await?)
    }

    /// Sets the table's TTL (bootstrap only, needs the admin user).
    pub async fn set_ttl_days(&self, table: Table, days: u32) -> Result<()> {
        let sql = format!(
            "ALTER TABLE {} MODIFY TTL toDateTime({}) + INTERVAL {days} DAY",
            table.name(),
            table.time_column()
        );
        self.client.query(&sql).execute().await?;
        Ok(())
    }

    // ------------------------------------------------------------ writes

    async fn insert_rows<T>(&self, table: &str, rows: &[T]) -> Result<()>
    where
        T: clickhouse::RowOwned + clickhouse::RowWrite,
    {
        if rows.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert::<T>(table).await?;
        for r in rows {
            insert.write(r).await?;
        }
        insert.end().await?;
        Ok(())
    }

    pub async fn insert_spans(&self, rows: &[SpanRow]) -> Result<()> {
        self.insert_rows("spans", rows).await
    }

    pub async fn insert_logs(&self, rows: &[LogRow]) -> Result<()> {
        self.insert_rows("logs", rows).await
    }

    pub async fn insert_events(&self, rows: &[EventRow]) -> Result<()> {
        self.insert_rows("events", rows).await
    }

    pub async fn insert_metrics(&self, rows: &[MetricRow]) -> Result<()> {
        self.insert_rows("metrics", rows).await
    }

    /// First and last point of every infrastructure series in `[from, to]`.
    pub async fn infra_series(&self, s: &Scope, from: i64, to: i64, limit: u32) -> Result<Vec<InfraSeriesRow>> {
        let q = self.client.query(&infra_series_sql());
        Ok(bind_scope(q, s).bind(from).bind(to).bind(limit).fetch_all().await?)
    }

    /// Infrastructure metrics in `[base, to)` per bucket of `bucket_ns` (see [`InfraBucketRow`]).
    pub async fn infra_buckets(
        &self,
        s: &Scope,
        base: i64,
        to: i64,
        bucket_ns: i64,
        limit: u32,
    ) -> Result<Vec<InfraBucketRow>> {
        let q = self.client.query(&infra_buckets_sql()).bind(base).bind(bucket_ns);
        Ok(bind_scope(q, s).bind(base).bind(to).bind(limit).fetch_all().await?)
    }

    /// Request latency per service instance: `[now_from, to)` against the baseline `[from, now_from)`.
    pub async fn instance_latency(
        &self,
        s: &Scope,
        from: i64,
        now_from: i64,
        to: i64,
        limit: u32,
    ) -> Result<Vec<InstanceLatencyRow>> {
        let q = self.client.query(&instance_latency_sql()).bind(now_from).bind(now_from).bind(now_from).bind(now_from);
        Ok(bind_scope(q, s).bind(from).bind(to).bind(limit).fetch_all().await?)
    }

    /// Addresses services connect to databases and queues at in `[from, to)` (see [`ResourceAddressRow`]).
    pub async fn resource_addresses(
        &self,
        s: &Scope,
        from: i64,
        to: i64,
        limit: u32,
    ) -> Result<Vec<ResourceAddressRow>> {
        let q = self.client.query(&resource_addresses_sql());
        Ok(bind_scope(q, s).bind(from).bind(to).bind(limit).fetch_all().await?)
    }

    // ------------------------------------------------------------ reads

    pub async fn service_summaries(&self, s: &Scope, from: i64, to: i64) -> Result<Vec<ServiceSummaryRow>> {
        let q = self.client.query(&format!(
            "SELECT service_id, any(service_name) AS service_name,
                    countIf(kind IN (2, 5) OR parent_span_id = '') AS requests,
                    countIf((kind IN (2, 5) OR parent_span_id = '') AND status_code = 2) AS errors,
                    quantileIf(0.95)(toFloat64(duration_ns), kind IN (2, 5) OR parent_span_id = '') AS p95_ns,
                    max(end_time) AS last_seen,
                    uniqExact(service_instance_id) AS instances,
                    argMax(service_version, end_time) AS version
             FROM spans FINAL
             WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
             GROUP BY service_id ORDER BY service_name"
        ));
        Ok(bind_scope(q, s).bind(from).bind(to).fetch_all().await?)
    }

    pub async fn log_services(&self, s: &Scope, from: i64, to: i64) -> Result<Vec<LogServiceRow>> {
        let q = self.client.query(&format!(
            "SELECT service_id, any(service_name) AS service_name, count() AS logs,
                    countIf(severity_number >= 17) AS error_logs, max(time) AS last_seen
             FROM logs FINAL
             WHERE {SCOPE_WHERE} AND time >= fromUnixTimestamp64Nano(?) AND time < fromUnixTimestamp64Nano(?)
             GROUP BY service_id"
        ));
        Ok(bind_scope(q, s).bind(from).bind(to).fetch_all().await?)
    }

    /// Observed service-to-service calls: a server/consumer span whose parent
    /// span belongs to a different service. Parents may start up to 5 min
    /// before the window.
    pub async fn call_edges(&self, s: &Scope, from: i64, to: i64) -> Result<Vec<CallEdgeRow>> {
        let q = self.client.query(&format!(
            "SELECT p.service_id AS source_service_id, c.service_id AS target_service_id,
                    count() AS calls, countIf(c.status_code = 2) AS errors, max(c.end_time) AS last_seen
             FROM
               (SELECT trace_id, span_id, service_id FROM spans FINAL
                WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)) AS p
             INNER JOIN
               (SELECT trace_id, parent_span_id, service_id, status_code, end_time FROM spans FINAL
                WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
                  AND kind IN (2, 5) AND parent_span_id != '') AS c
             ON p.trace_id = c.trace_id AND p.span_id = c.parent_span_id
             WHERE p.service_id != c.service_id
             GROUP BY source_service_id, target_service_id"
        ));
        let q = bind_scope(q, s).bind(from.saturating_sub(SPAN_LOOKBACK_NS)).bind(to);
        Ok(bind_scope(q, s).bind(from).bind(to).fetch_all().await?)
    }

    /// Inferred dependencies on uninstrumented resources (databases etc.) and external HTTP
    /// services. A host counts as external only when no call to it in the window was answered by
    /// an instrumented service (a server/consumer child span, which may start up to 5 min later).
    /// Decided per host, not per call: services export in batches, so the newest calls to an
    /// internal service routinely arrive before their answers.
    pub async fn resource_edges(&self, s: &Scope, from: i64, to: i64) -> Result<Vec<ResourceEdgeRow>> {
        let q = self.client.query(&format!(
            "SELECT service_id, target_entity_id, any(target_name) AS target_name, any(target_kind) AS target_kind,
                    count() AS calls, countIf(status_code = 2) AS errors, max(end_time) AS last_seen
             FROM spans FINAL
             WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
               AND target_entity_id != ''
               AND (spans.target_kind != 'external' OR target_entity_id NOT IN (
                 SELECT target_entity_id FROM spans FINAL
                 WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
                   AND target_kind = 'external'
                   AND (trace_id, span_id) IN (
                     SELECT trace_id, parent_span_id FROM spans FINAL
                     WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
                       AND kind IN (2, 5) AND parent_span_id != '')))
             GROUP BY service_id, target_entity_id"
        ));
        let q = bind_scope(q, s).bind(from).bind(to);
        let q = bind_scope(q, s).bind(from).bind(to);
        Ok(bind_scope(q, s).bind(from).bind(to.saturating_add(SPAN_LOOKBACK_NS)).fetch_all().await?)
    }

    /// Requests that entered a service from outside: root server spans (no instrumented caller).
    pub async fn external_client_edges(&self, s: &Scope, from: i64, to: i64) -> Result<Vec<ExternalClientEdgeRow>> {
        let q = self.client.query(&format!(
            "SELECT service_id, count() AS calls, countIf(status_code = 2) AS errors, max(end_time) AS last_seen
             FROM spans FINAL
             WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
               AND kind = 2 AND parent_span_id = ''
             GROUP BY service_id"
        ));
        Ok(bind_scope(q, s).bind(from).bind(to).fetch_all().await?)
    }

    pub async fn traces(&self, s: &Scope, tq: &TraceQuery) -> Result<Vec<TraceSummaryRow>> {
        let sql = traces_sql(tq);
        let mut q = bind_scope(self.client.query(&sql), s).bind(tq.from_ns).bind(tq.to_ns);
        if let Some(svc) = &tq.service_id {
            q = bind_scope(q, s).bind(svc.as_str()).bind(tq.from_ns).bind(tq.to_ns);
        }
        if let Some((t, id)) = &tq.before {
            q = q.bind(*t).bind(id.as_str());
        }
        Ok(q.bind(tq.limit).fetch_all().await?)
    }

    pub async fn trace_spans(&self, s: &Scope, trace_id: &str, limit: u32) -> Result<Vec<SpanRow>> {
        let q = self.client.query(&format!(
            "SELECT ?fields FROM spans FINAL WHERE {SCOPE_WHERE} AND trace_id = ? ORDER BY start_time LIMIT ?"
        ));
        Ok(bind_scope(q, s).bind(trace_id).bind(limit).fetch_all().await?)
    }

    pub async fn logs(&self, s: &Scope, lq: &LogQuery) -> Result<Vec<LogRow>> {
        let sql = logs_sql(lq);
        let mut q = bind_scope(self.client.query(&sql), s).bind(lq.from_ns).bind(lq.to_ns);
        if let Some(v) = &lq.service_id {
            q = q.bind(v.as_str());
        }
        if let Some(v) = lq.min_severity {
            q = q.bind(v);
        }
        if let Some(v) = &lq.trace_id {
            q = q.bind(v.as_str());
        }
        if let Some(v) = &lq.text {
            q = q.bind(v.as_str());
        }
        if let Some(v) = lq.template_id {
            q = q.bind(v);
        }
        if let Some((t, id)) = &lq.before {
            q = q.bind(*t).bind(id.as_str());
        }
        Ok(q.bind(lq.limit).fetch_all().await?)
    }

    pub async fn events(
        &self,
        s: &Scope,
        from: i64,
        to: i64,
        trace_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<EventRow>> {
        let sql = events_sql(trace_id.is_some());
        let mut q = bind_scope(self.client.query(&sql), s).bind(from).bind(to);
        if let Some(t) = trace_id {
            q = q.bind(t);
        }
        Ok(q.bind(limit).fetch_all().await?)
    }

    // ------------------------------------------------------------ archive (karbird-archiver)

    /// Services with logs in `[from, to)`, per scope; all scopes unless `scope` is given.
    pub async fn log_groups(&self, from: i64, to: i64, scope: Option<&Scope>) -> Result<Vec<LogGroupRow>> {
        let q = self.client.query(&log_groups_sql(scope.is_some())).bind(from).bind(to);
        let q = match scope {
            Some(s) => bind_scope(q, s),
            None => q,
        };
        Ok(q.fetch_all().await?)
    }

    /// Distinct log rows in `[from, to)`; all scopes unless `scope` is given.
    pub async fn count_logs(&self, from: i64, to: i64, scope: Option<&Scope>) -> Result<u64> {
        let q = self.client.query(&count_logs_sql(scope.is_some())).bind(from).bind(to);
        let q = match scope {
            Some(s) => bind_scope(q, s),
            None => q,
        };
        Ok(q.fetch_one::<u64>().await?)
    }

    /// Time of the oldest stored log, or `None` when there are none.
    pub async fn earliest_log_time(&self) -> Result<Option<i64>> {
        let row: EarliestRow =
            self.client.query("SELECT min(time) AS earliest, count() AS rows FROM logs").fetch_one().await?;
        Ok((row.rows > 0).then_some(row.earliest))
    }

    /// Every distinct log row of one service in `[from, to)`, oldest first.
    pub async fn logs_for_archive(&self, s: &Scope, service_id: &str, from: i64, to: i64) -> Result<Vec<LogRow>> {
        let q = self.client.query(&format!(
            "SELECT ?fields FROM logs FINAL
             WHERE {SCOPE_WHERE} AND service_id = ? AND time >= fromUnixTimestamp64Nano(?) AND time < fromUnixTimestamp64Nano(?)
             ORDER BY time, log_id"
        ));
        Ok(bind_scope(q, s).bind(service_id).bind(from).bind(to).fetch_all().await?)
    }

    /// Spans overlapping `[from, to)` (start < to AND end >= from).
    pub async fn timeline_spans(&self, s: &Scope, from: i64, to: i64, limit: u32) -> Result<Vec<TimelineSpanRow>> {
        let q = self.client.query(&format!(
            "SELECT {TIMELINE_SPAN_COLUMNS}
             FROM spans FINAL
             WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
               AND end_time >= fromUnixTimestamp64Nano(?)
             ORDER BY start_time, trace_id, span_id LIMIT ?"
        ));
        // Spans are bounded in length by the 5 min look-back used for overlap.
        Ok(bind_scope(q, s)
            .bind(from.saturating_sub(SPAN_LOOKBACK_NS))
            .bind(to)
            .bind(from)
            .bind(limit)
            .fetch_all()
            .await?)
    }

    pub async fn timeline_logs(&self, s: &Scope, from: i64, to: i64, limit: u32) -> Result<Vec<TimelineLogRow>> {
        let q = self.client.query(&format!(
            "SELECT {TIMELINE_LOG_COLUMNS}
             FROM logs FINAL
             WHERE {SCOPE_WHERE} AND time >= fromUnixTimestamp64Nano(?) AND time < fromUnixTimestamp64Nano(?)
             ORDER BY time, log_id LIMIT ?"
        ));
        Ok(bind_scope(q, s).bind(from).bind(to).bind(limit).fetch_all().await?)
    }

    // Live queries select by ingestion time `(after, upto]`, so late-arriving telemetry is
    // delivered too. `event_from` bounds the scan to recent partitions. No FINAL: the engine
    // deduplicates by item ID.

    pub async fn live_spans(
        &self,
        s: &Scope,
        event_from: i64,
        after: i64,
        upto: i64,
        limit: u32,
    ) -> Result<Vec<TimelineSpanRow>> {
        let q = self.client.query(&format!(
            "SELECT {TIMELINE_SPAN_COLUMNS} FROM spans
             WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?)
               AND ingested_at > fromUnixTimestamp64Nano(?) AND ingested_at <= fromUnixTimestamp64Nano(?)
             ORDER BY ingested_at LIMIT ?"
        ));
        Ok(bind_scope(q, s).bind(event_from).bind(after).bind(upto).bind(limit).fetch_all().await?)
    }

    pub async fn live_logs(
        &self,
        s: &Scope,
        event_from: i64,
        after: i64,
        upto: i64,
        limit: u32,
    ) -> Result<Vec<TimelineLogRow>> {
        let q = self.client.query(&format!(
            "SELECT {TIMELINE_LOG_COLUMNS} FROM logs
             WHERE {SCOPE_WHERE} AND time >= fromUnixTimestamp64Nano(?)
               AND ingested_at > fromUnixTimestamp64Nano(?) AND ingested_at <= fromUnixTimestamp64Nano(?)
             ORDER BY ingested_at LIMIT ?"
        ));
        Ok(bind_scope(q, s).bind(event_from).bind(after).bind(upto).bind(limit).fetch_all().await?)
    }

    pub async fn live_events(
        &self,
        s: &Scope,
        event_from: i64,
        after: i64,
        upto: i64,
        limit: u32,
    ) -> Result<Vec<EventRow>> {
        let q = self.client.query(&format!(
            "SELECT ?fields FROM events
             WHERE {SCOPE_WHERE} AND event_time >= fromUnixTimestamp64Nano(?)
               AND ingested_at > fromUnixTimestamp64Nano(?) AND ingested_at <= fromUnixTimestamp64Nano(?)
             ORDER BY ingested_at LIMIT ?"
        ));
        Ok(bind_scope(q, s).bind(event_from).bind(after).bind(upto).bind(limit).fetch_all().await?)
    }
}

const TIMELINE_SPAN_COLUMNS: &str = "trace_id, span_id, parent_span_id, service_id, service_name, target_entity_id, \
     name, kind, start_time, end_time, status_code, ingested_at";
const TIMELINE_LOG_COLUMNS: &str = "log_id, service_id, service_name, time, severity_number, trace_id, ingested_at";

/// Split a migration file into statements on `;` at line ends, skipping comments.
fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") || trimmed.is_empty() {
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
        if trimmed.ends_with(';') {
            let stmt = cur.trim().trim_end_matches(';').trim().to_owned();
            if !stmt.is_empty() {
                out.push(stmt);
            }
            cur.clear();
        }
    }
    let rest = cur.trim();
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn splits_migration_statements() {
        let sql = "-- c\nCREATE TABLE a (x UInt8);\n\nCREATE TABLE b\n(y UInt8);\n";
        let s = split_statements(sql);
        assert_eq!(s.len(), 2);
        assert!(s[1].starts_with("CREATE TABLE b"));
    }

    #[test]
    fn split_statements_edge_cases() {
        assert!(split_statements("").is_empty());
        assert!(split_statements("-- only a comment\n\n   \n").is_empty());
        assert!(split_statements(";\n  ;  \n").is_empty(), "empty statements are dropped");
        assert_eq!(split_statements("SELECT 1"), ["SELECT 1"], "trailing statement without ;");
        assert_eq!(split_statements("SELECT 1;\r\nSELECT 2;\r\n"), ["SELECT 1", "SELECT 2"], "CRLF files");
        // Indented comments are skipped too; `;` inside a line does not split.
        let s = split_statements("  -- x\nSELECT 'a;b' AS v;\n");
        assert_eq!(s, ["SELECT 'a;b' AS v"]);
    }

    #[test]
    fn real_clickhouse_migrations_split_into_statements() {
        for sql in [
            include_str!("../../../infra/migrations/clickhouse/0001_telemetry.sql"),
            include_str!("../../../infra/migrations/clickhouse/0002_log_templates_and_codecs.sql"),
            include_str!("../../../infra/migrations/clickhouse/0003_metrics.sql"),
            include_str!("../../../infra/migrations/clickhouse/0004_metrics_container_identity.sql"),
        ] {
            let stmts = split_statements(sql);
            assert!(!stmts.is_empty());
            for s in &stmts {
                assert!(!s.ends_with(';') && !s.trim().is_empty(), "{s}");
                assert!(!s.lines().any(|l| l.trim_start().starts_with("--")), "comments stripped: {s}");
            }
        }
    }

    /// `?` placeholders, excluding the crate's `?fields` column expansion.
    fn placeholders(sql: &str) -> usize {
        sql.match_indices('?').filter(|(i, _)| !sql[*i + 1..].starts_with("fields")).count()
    }

    fn scoped_where_count(sql: &str) -> usize {
        sql.matches("tenant_id = ? AND project_id = ? AND environment_id = ?").count()
    }

    #[test]
    fn log_sql_placeholders_match_binds_for_every_filter_combination() {
        for mask in 0u8..64 {
            let lq = LogQuery {
                from_ns: 1,
                to_ns: 2,
                service_id: (mask & 1 != 0).then(|| "svc".into()),
                min_severity: (mask & 2 != 0).then_some(17),
                trace_id: (mask & 4 != 0).then(|| "a".repeat(32)),
                text: (mask & 8 != 0).then(|| "needle".into()),
                before: (mask & 16 != 0).then(|| (5, "id".into())),
                template_id: (mask & 32 != 0).then_some(u64::MAX),
                limit: 10,
            };
            let sql = logs_sql(&lq);
            // scope (3) + range (2) + one per optional filter + cursor (2) + limit (1)
            let expected = 3
                + 2
                + usize::from(lq.service_id.is_some())
                + usize::from(lq.min_severity.is_some())
                + usize::from(lq.trace_id.is_some())
                + usize::from(lq.text.is_some())
                + usize::from(lq.template_id.is_some())
                + 2 * usize::from(lq.before.is_some())
                + 1;
            assert_eq!(placeholders(&sql), expected, "mask {mask}: {sql}");
            assert_eq!(scoped_where_count(&sql), 1);
            assert!(sql.contains("?fields"));
            assert!(sql.trim_end().ends_with("LIMIT ?"));
        }
    }

    #[test]
    fn trace_sql_placeholders_match_binds_and_scope_the_subquery() {
        for mask in 0u8..8 {
            let tq = TraceQuery {
                from_ns: 1,
                to_ns: 2,
                service_id: (mask & 1 != 0).then(|| "svc".into()),
                only_errors: mask & 2 != 0,
                before: (mask & 4 != 0).then(|| (5, "t".into())),
                limit: 10,
            };
            let sql = traces_sql(&tq);
            let svc = usize::from(tq.service_id.is_some());
            let expected = 3 + 2 + svc * (3 + 1 + 2) + 2 * usize::from(tq.before.is_some()) + 1;
            assert_eq!(placeholders(&sql), expected, "mask {mask}: {sql}");
            // The service subquery is scoped as well, so it cannot pull trace IDs from another tenant.
            assert_eq!(scoped_where_count(&sql), 1 + svc, "{sql}");
            assert_eq!(sql.contains("HAVING"), tq.only_errors || tq.before.is_some());
            assert_eq!(sql.contains("error_count > 0"), tq.only_errors);
        }
    }

    #[test]
    fn archive_sql_is_scoped_only_when_asked_and_binds_match() {
        for scoped in [false, true] {
            for sql in [log_groups_sql(scoped), count_logs_sql(scoped)] {
                assert_eq!(placeholders(&sql), 2 + if scoped { 3 } else { 0 }, "{sql}");
                assert_eq!(scoped_where_count(&sql), usize::from(scoped), "{sql}");
                assert!(sql.contains("FINAL"), "archive reads see deduplicated rows: {sql}");
            }
        }
    }

    #[test]
    fn infrastructure_timeline_sql_is_scoped_and_binds_match() {
        let sql = infra_buckets_sql();
        // base and bucket size, scope, base and to, limit (as bound in `infra_buckets`).
        assert_eq!(placeholders(&sql), 2 + 3 + 2 + 1, "{sql}");
        assert_eq!(scoped_where_count(&sql), 1);
        assert!(sql.contains("FROM metrics FINAL"));
        for name in INFRA_TIMELINE_METRIC_NAMES {
            assert!(INFRA_METRIC_NAMES.contains(name), "{name} is an infrastructure metric");
            assert!(sql.contains(&format!("'{name}'")));
        }
    }

    #[test]
    fn infrastructure_sql_is_scoped_and_binds_match() {
        let series = infra_series_sql();
        assert_eq!(placeholders(&series), 3 + 2 + 1, "{series}");
        assert_eq!(scoped_where_count(&series), 1);
        assert!(series.contains("FROM metrics FINAL"));
        for name in INFRA_METRIC_NAMES {
            // Embedded as SQL literals, so they must never need escaping.
            assert!(
                name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_'),
                "{name}"
            );
            assert!(series.contains(&format!("'{name}'")));
        }
        let latency = instance_latency_sql();
        assert_eq!(placeholders(&latency), 4 + 3 + 2 + 1, "{latency}");
        assert_eq!(scoped_where_count(&latency), 1);
        assert!(series.contains("container_image") && series.contains("container_service"));
    }

    #[test]
    fn resource_address_sql_is_scoped_reads_only_resource_calls_and_binds_match() {
        let sql = resource_addresses_sql();
        assert_eq!(placeholders(&sql), 3 + 2 + 1, "{sql}");
        assert_eq!(scoped_where_count(&sql), 1);
        assert!(sql.contains("FROM spans FINAL"));
        assert!(sql.contains("kind IN (3, 4)"), "only outgoing calls: {sql}");
        assert!(sql.contains("target_kind IN ('database', 'queue')"), "{sql}");
        // The statement text is never read: only where the call went.
        assert!(!sql.contains("db.statement") && !sql.contains("db.query.text"), "{sql}");
        // ClickHouse resolves an alias before the column: `any(target_kind) AS target_kind`
        // would put an aggregate into WHERE (ILLEGAL_AGGREGATION).
        for column in ["target_kind", "target_name", "target_entity_id"] {
            assert!(!sql.contains(&format!("AS {column}")), "{column} is aliased: {sql}");
        }
    }

    #[test]
    fn event_sql_placeholders_match_binds() {
        assert_eq!(placeholders(&events_sql(false)), 3 + 2 + 1);
        assert_eq!(placeholders(&events_sql(true)), 3 + 2 + 1 + 1);
        assert_eq!(scoped_where_count(&events_sql(true)), 1);
    }

    #[test]
    fn user_supplied_strings_never_reach_the_sql_text() {
        let evil = "x' OR 1=1; DROP TABLE logs; --";
        let lq = LogQuery {
            service_id: Some(evil.into()),
            trace_id: Some(evil.into()),
            text: Some(evil.into()),
            before: Some((1, evil.into())),
            ..LogQuery::default()
        };
        let tq = TraceQuery { service_id: Some(evil.into()), before: Some((1, evil.into())), ..TraceQuery::default() };
        for sql in [logs_sql(&lq), traces_sql(&tq)] {
            assert!(!sql.contains("DROP") && !sql.contains("1=1") && !sql.contains('\''), "{sql}");
        }
    }

    #[test]
    fn timeline_column_lists_match_row_structs() {
        let span_cols: Vec<_> = TIMELINE_SPAN_COLUMNS.split(',').map(str::trim).collect();
        assert_eq!(
            span_cols,
            [
                "trace_id",
                "span_id",
                "parent_span_id",
                "service_id",
                "service_name",
                "target_entity_id",
                "name",
                "kind",
                "start_time",
                "end_time",
                "status_code",
                "ingested_at"
            ]
        );
        let log_cols: Vec<_> = TIMELINE_LOG_COLUMNS.split(',').map(str::trim).collect();
        assert_eq!(
            log_cols,
            ["log_id", "service_id", "service_name", "time", "severity_number", "trace_id", "ingested_at"]
        );
    }

    #[test]
    fn span_lookback_does_not_overflow() {
        assert_eq!(i64::MIN.saturating_sub(SPAN_LOOKBACK_NS), i64::MIN);
        assert_eq!(1_000_000_000_000i64.saturating_sub(SPAN_LOOKBACK_NS), 700_000_000_000);
    }

    fn store() -> ChStore {
        ChStore::new(&ChConfig {
            url: "http://127.0.0.1:1".into(),
            database: "karbird".into(),
            user: "u".into(),
            password: "p".into(),
        })
    }

    #[tokio::test]
    async fn ensure_user_rejects_injection_before_any_query() {
        let s = store();
        for (user, pw) in [
            ("karbird_api", "pw' ; DROP USER default; --"),
            ("x; DROP DATABASE karbird", "pw"),
            ("", "pw"),
            ("user", ""),
            ("user name", "pw"),
            ("user", "p\\w"),
            ("ünïcode", "pw"),
        ] {
            match s.ensure_user(user, pw, &[]).await {
                Err(StorageError::Other(m)) => assert!(m.contains("charset"), "{m}"),
                other => panic!("{user:?}/{pw:?} must be refused before any query, got {other:?}"),
            }
        }
    }

    #[test]
    fn debug_output_does_not_expose_credentials() {
        assert_eq!(format!("{:?}", store()), "ChStore { .. }");
    }
}
