//! ClickHouse: telemetry rows, batched inserts and scoped analytical queries.
//! Every query binds tenant/project/environment; there is no raw-SQL entry point.

use clickhouse::{Client, Row};
use serde::{Deserialize, Serialize};

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

// ---------------------------------------------------------------- query results

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

#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub from_ns: i64,
    pub to_ns: i64,
    pub service_id: Option<String>,
    pub min_severity: Option<u8>,
    pub trace_id: Option<String>,
    pub text: Option<String>,
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
        let q = bind_scope(q, s).bind(from - 300_000_000_000).bind(to);
        Ok(bind_scope(q, s).bind(from).bind(to).fetch_all().await?)
    }

    /// Inferred dependencies on uninstrumented resources (databases etc.).
    pub async fn resource_edges(&self, s: &Scope, from: i64, to: i64) -> Result<Vec<ResourceEdgeRow>> {
        let q = self.client.query(&format!(
            "SELECT service_id, target_entity_id, any(target_name) AS target_name, any(target_kind) AS target_kind,
                    count() AS calls, countIf(status_code = 2) AS errors, max(end_time) AS last_seen
             FROM spans FINAL
             WHERE {SCOPE_WHERE} AND start_time >= fromUnixTimestamp64Nano(?) AND start_time < fromUnixTimestamp64Nano(?)
               AND target_entity_id != ''
             GROUP BY service_id, target_entity_id"
        ));
        Ok(bind_scope(q, s).bind(from).bind(to).fetch_all().await?)
    }

    pub async fn traces(&self, s: &Scope, tq: &TraceQuery) -> Result<Vec<TraceSummaryRow>> {
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
        if lq.before.is_some() {
            sql.push_str(" AND (time, log_id) < (fromUnixTimestamp64Nano(?), ?)");
        }
        sql.push_str(" ORDER BY time DESC, log_id DESC LIMIT ?");
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
        let mut sql = format!(
            "SELECT ?fields FROM events FINAL
             WHERE {SCOPE_WHERE} AND event_time >= fromUnixTimestamp64Nano(?) AND event_time < fromUnixTimestamp64Nano(?)"
        );
        if trace_id.is_some() {
            sql.push_str(" AND trace_id = ?");
        }
        sql.push_str(" ORDER BY event_time DESC, event_id DESC LIMIT ?");
        let mut q = bind_scope(self.client.query(&sql), s).bind(from).bind(to);
        if let Some(t) = trace_id {
            q = q.bind(t);
        }
        Ok(q.bind(limit).fetch_all().await?)
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
        Ok(bind_scope(q, s).bind(from - 300_000_000_000).bind(to).bind(from).bind(limit).fetch_all().await?)
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
    use super::split_statements;

    #[test]
    fn splits_migration_statements() {
        let sql = "-- c\nCREATE TABLE a (x UInt8);\n\nCREATE TABLE b\n(y UInt8);\n";
        let s = split_statements(sql);
        assert_eq!(s.len(), 2);
        assert!(s[1].starts_with("CREATE TABLE b"));
    }
}
