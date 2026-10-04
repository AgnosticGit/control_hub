//! Archiving one window: per service, read the logs from ClickHouse, encode a segment with the
//! service's current dictionary, verify it, store it, record it in the catalog, and grow the
//! dictionary for the next segment. Nothing is deleted from ClickHouse (its TTL does that).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use controlhub_archive::index::{SegmentIndex, segment_index};
use controlhub_archive::{Dictionary, LogRecord, dict::MAX_DICT_BYTES};
use controlhub_domain::{EnvironmentId, ProjectId, Scope, TenantId};
use controlhub_storage::archive::{ArchiveObjects, SegmentRow, dict_key, segment_key, sha256_hex};
use controlhub_storage::ch::{ChStore, LogRow};
use controlhub_storage::pg::PgStore;
use serde::Serialize;

#[derive(Debug, Default)]
pub struct Metrics {
    pub windows_archived: AtomicU64,
    pub windows_lost: AtomicU64,
    pub windows_rearchived: AtomicU64,
    pub segments_written: AtomicU64,
    pub rows_archived: AtomicU64,
    pub source_bytes: AtomicU64,
    pub stored_bytes: AtomicU64,
    pub dictionaries_written: AtomicU64,
    pub failures: AtomicU64,
    pub gc_segments_deleted: AtomicU64,
    pub last_window_end: AtomicU64,
}

impl Metrics {
    pub fn render(&self) -> String {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        format!(
            "controlhub_archiver_windows_total{{result=\"archived\"}} {}\n\
             controlhub_archiver_windows_total{{result=\"lost\"}} {}\n\
             controlhub_archiver_windows_total{{result=\"rearchived\"}} {}\n\
             controlhub_archiver_segments_written_total {}\n\
             controlhub_archiver_rows_archived_total {}\n\
             controlhub_archiver_source_bytes_total {}\n\
             controlhub_archiver_stored_bytes_total {}\n\
             controlhub_archiver_dictionaries_written_total {}\n\
             controlhub_archiver_failures_total {}\n\
             controlhub_archiver_gc_segments_deleted_total {}\n\
             controlhub_archiver_last_window_end_ns {}\n",
            g(&self.windows_archived),
            g(&self.windows_lost),
            g(&self.windows_rearchived),
            g(&self.segments_written),
            g(&self.rows_archived),
            g(&self.source_bytes),
            g(&self.stored_bytes),
            g(&self.dictionaries_written),
            g(&self.failures),
            g(&self.gc_segments_deleted),
            g(&self.last_window_end),
        )
    }
}

#[derive(Clone)]
pub struct Ctx {
    pub pg: PgStore,
    pub ch: ChStore,
    pub objects: ArchiveObjects,
    pub zstd_level: i32,
    pub metrics: Arc<Metrics>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub windows: u64,
    pub segments: u64,
    pub rows: u64,
    pub source_bytes: u64,
    pub stored_bytes: u64,
}

impl Summary {
    fn add(&mut self, o: Summary) {
        self.windows += o.windows;
        self.segments += o.segments;
        self.rows += o.rows;
        self.source_bytes += o.source_bytes;
        self.stored_bytes += o.stored_bytes;
    }
}

pub fn record_of(r: LogRow) -> LogRecord {
    LogRecord {
        service_name: r.service_name,
        service_instance_id: r.service_instance_id,
        log_id: r.log_id,
        time: r.time,
        observed_time: r.observed_time,
        severity_number: r.severity_number,
        severity_text: r.severity_text,
        body: r.body,
        trace_id: r.trace_id,
        span_id: r.span_id,
        attributes: r.attributes,
        resource_attributes: r.resource_attributes,
        batch_id: r.batch_id,
        ingested_at: r.ingested_at,
    }
}

pub fn parse_scope(tenant: &str, project: &str, env: &str) -> anyhow::Result<Scope> {
    Ok(Scope {
        tenant_id: TenantId::parse(tenant)?,
        project_id: ProjectId::parse(project)?,
        environment_id: EnvironmentId::parse(env)?,
    })
}

/// Encodes and verifies a segment off the async runtime, and builds its catalog search index.
/// The decoded rows must equal the input before anything is stored.
pub fn build_segment(
    mut records: Vec<LogRecord>,
    dict: Option<&Dictionary>,
    level: i32,
) -> anyhow::Result<(controlhub_archive::Encoded, SegmentIndex)> {
    let enc = controlhub_archive::encode(&records, dict, level)?;
    let mut back = controlhub_archive::decode(&enc.bytes, dict)?;
    records.sort();
    back.sort();
    anyhow::ensure!(back == records, "segment verification failed: decoded rows differ from the source");
    Ok((enc, segment_index(&records)))
}

async fn load_dictionary(ctx: &Ctx, s: &Scope, service_id: &str) -> anyhow::Result<Option<Dictionary>> {
    let Some((version, key)) = ctx.pg.latest_archive_dictionary(s, service_id).await? else { return Ok(None) };
    let bytes = ctx.objects.get(&key).await?;
    Ok(Some(Dictionary::from_bytes(version, &bytes)?))
}

/// Archives one service's logs of `[start, end)`. Returns the segment's summary.
pub async fn archive_service(ctx: &Ctx, s: &Scope, service_id: &str, start: i64, end: i64) -> anyhow::Result<Summary> {
    let rows = ctx.ch.logs_for_archive(s, service_id, start, end).await?;
    if rows.is_empty() {
        return Ok(Summary::default());
    }
    let dict = load_dictionary(ctx, s, service_id).await?;
    let records: Vec<LogRecord> = rows.into_iter().map(record_of).collect();
    let level = ctx.zstd_level;
    let ((enc, index), dict) =
        tokio::task::spawn_blocking(move || build_segment(records, dict.as_ref(), level).map(|e| (e, dict))).await??;

    let sha = sha256_hex(&enc.bytes);
    let key = segment_key(s, service_id, start, &sha);
    ctx.objects.put(&key, enc.bytes).await?;
    let stored = ctx.objects.get(&key).await?;
    anyhow::ensure!(sha256_hex(&stored) == sha, "segment {key} read back with a different checksum");

    let st = &enc.stats;
    let row = SegmentRow {
        tenant_id: s.tenant_id.to_string(),
        project_id: s.project_id.to_string(),
        environment_id: s.environment_id.to_string(),
        service_id: service_id.to_owned(),
        window_start_ns: start,
        window_end_ns: end,
        min_time_ns: st.min_time,
        max_time_ns: st.max_time,
        rows: i64::try_from(st.rows)?,
        source_bytes: i64::try_from(st.source_bytes)?,
        stored_bytes: i64::try_from(st.stored_bytes)?,
        templates: i32::try_from(st.templates)?,
        dict_version: dict.as_ref().map_or(0, |d| i32::try_from(d.version()).unwrap_or(0)),
        object_key: key,
        sha256: sha,
        dict_key: None,
    };
    if let Some(old) = ctx.pg.upsert_archive_segment(&row, &index).await?.filter(|old| *old != row.object_key) {
        ctx.objects.delete(&old).await?;
    }

    // The object exists before the catalog points at it. A lost race leaves an unreferenced
    // object behind (it may even be the winner's, when both built identical bytes), never a
    // catalog row without its object.
    if let Some(next) = Dictionary::evolve(dict.as_ref(), &enc.template_counts, &enc.string_counts, MAX_DICT_BYTES) {
        let dkey = dict_key(s, service_id, next.version(), &sha256_hex(next.as_bytes()));
        ctx.objects.put(&dkey, next.as_bytes().to_vec()).await?;
        if ctx.pg.insert_archive_dictionary(s, service_id, &next, &dkey).await? {
            ctx.metrics.dictionaries_written.fetch_add(1, Ordering::Relaxed);
        }
    }

    let m = &ctx.metrics;
    m.segments_written.fetch_add(1, Ordering::Relaxed);
    m.rows_archived.fetch_add(st.rows, Ordering::Relaxed);
    m.source_bytes.fetch_add(st.source_bytes, Ordering::Relaxed);
    m.stored_bytes.fetch_add(st.stored_bytes, Ordering::Relaxed);
    tracing::debug!(service_id, start, rows = st.rows, stored = st.stored_bytes, "segment archived");
    Ok(Summary { windows: 0, segments: 1, rows: st.rows, source_bytes: st.source_bytes, stored_bytes: st.stored_bytes })
}

/// Archives every service with logs in `[start, end)` (all scopes, or one). Fails as a whole
/// when any service fails, so the window is retried and never recorded as done half-way.
pub async fn archive_window(ctx: &Ctx, start: i64, end: i64, scope: Option<&Scope>) -> anyhow::Result<Summary> {
    let groups = ctx.ch.log_groups(start, end, scope).await?;
    let mut total = Summary { windows: 1, ..Summary::default() };
    for g in groups {
        let s = parse_scope(&g.tenant_id, &g.project_id, &g.environment_id)?;
        total.add(archive_service(ctx, &s, &g.service_id, start, end).await?);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use controlhub_archive::index::bloom_may_contain;

    fn row(i: i64) -> LogRow {
        LogRow {
            tenant_id: "t".into(),
            project_id: "p".into(),
            environment_id: "e".into(),
            service_id: "svc".into(),
            service_name: "orders".into(),
            service_instance_id: "i-1".into(),
            log_id: format!("{i:032x}"),
            time: 1_000 + i,
            observed_time: 1_001 + i,
            severity_number: 9,
            severity_text: "INFO".into(),
            body: format!("order {i} created"),
            template_id: 0,
            trace_id: String::new(),
            span_id: String::new(),
            attributes: vec![("k".into(), "v".into())],
            resource_attributes: vec![],
            batch_id: "b".into(),
            ingested_at: 2_000 + i,
        }
    }

    #[test]
    fn record_conversion_keeps_every_archived_field() {
        let r = record_of(row(3));
        assert_eq!((r.time, r.observed_time, r.ingested_at), (1_003, 1_004, 2_003));
        assert_eq!(
            (r.body.as_str(), r.service_instance_id.as_str(), r.batch_id.as_str()),
            ("order 3 created", "i-1", "b")
        );
        assert_eq!(r.attributes, vec![("k".to_owned(), "v".to_owned())]);
    }

    #[test]
    fn built_segments_are_verified_and_decode_to_the_input() {
        let mut records: Vec<_> = (0..100).map(|i| record_of(row(i))).collect();
        records[7].trace_id = "ab".repeat(16);
        records[8].severity_number = 17;
        let (enc, index) = build_segment(records.clone(), None, 3).unwrap();
        assert_eq!(enc.stats.rows, 100);
        assert_eq!(index.template_ids.as_ref().map(Vec::len), Some(1), "\"order N created\" is one template");
        assert_eq!(index.max_severity, 17);
        assert!(bloom_may_contain(&index.trace_bloom, &"ab".repeat(16)));
        assert!(!bloom_may_contain(&index.trace_bloom, &"cd".repeat(16)), "one trace in a 64-bit filter");
        let mut back = controlhub_archive::decode(&enc.bytes, None).unwrap();
        back.sort();
        let mut want = records;
        want.sort();
        assert_eq!(back, want);
    }

    #[test]
    fn scopes_from_storage_are_validated() {
        assert!(parse_scope("t1", "p1", "e1").is_ok());
        assert!(parse_scope("", "p1", "e1").is_err());
        assert!(parse_scope("t1", "p 1", "e1").is_err());
    }

    #[test]
    fn metrics_render_every_series() {
        let m = Metrics::default();
        m.windows_lost.fetch_add(2, Ordering::Relaxed);
        m.stored_bytes.fetch_add(10, Ordering::Relaxed);
        let out = m.render();
        assert!(out.lines().any(|l| l == "controlhub_archiver_windows_total{result=\"lost\"} 2"), "{out}");
        assert!(out.lines().any(|l| l == "controlhub_archiver_stored_bytes_total 10"));
        assert_eq!(out.lines().count(), 11);
    }
}
