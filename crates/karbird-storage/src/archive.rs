//! Cold log archive (ADR-0015): segment and dictionary objects in S3-compatible storage, their
//! catalog in PostgreSQL, and the scoped reader the API uses for old time ranges.
//! Every catalog query binds tenant/project/environment; object keys are derived from them.

use std::collections::HashMap;
use std::sync::Arc;

use karbird_archive::index::{BLOOM_HASHES, SegmentIndex, bloom_probe};
use karbird_archive::{Dictionary, LogRecord};
use object_store::aws::AmazonS3Builder;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use tokio::sync::Mutex;

use crate::ch::{LogQuery, LogRow};
use crate::pg::PgStore;
use crate::{Result, Scope, StorageError};

/// The only archived signal so far.
pub const LOGS: &str = "logs";
/// A query never opens more segments than this; narrower ranges or filters page further.
pub const MAX_SEGMENTS_PER_QUERY: i64 = 400;
const DICT_CACHE_ENTRIES: usize = 256;

#[derive(Clone)]
pub struct ObjectConfig {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

impl std::fmt::Debug for ObjectConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectConfig")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

/// Segment and dictionary objects.
#[derive(Clone)]
pub struct ArchiveObjects {
    store: Arc<dyn ObjectStore>,
}

impl std::fmt::Debug for ArchiveObjects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveObjects").finish_non_exhaustive()
    }
}

fn obj_err(e: object_store::Error) -> StorageError {
    StorageError::Other(format!("archive object store: {e}"))
}

impl ArchiveObjects {
    pub fn s3(cfg: &ObjectConfig) -> Result<Self> {
        let store = AmazonS3Builder::new()
            .with_endpoint(&cfg.endpoint)
            .with_bucket_name(&cfg.bucket)
            .with_region(&cfg.region)
            .with_access_key_id(&cfg.access_key)
            .with_secret_access_key(&cfg.secret_key)
            .with_allow_http(cfg.endpoint.starts_with("http://"))
            .with_virtual_hosted_style_request(false)
            .build()
            .map_err(obj_err)?;
        Ok(Self { store: Arc::new(store) })
    }

    /// Process-local store for tests.
    pub fn in_memory() -> Self {
        Self { store: Arc::new(InMemory::new()) }
    }

    fn path(key: &str) -> Result<Path> {
        Path::parse(key).map_err(|e| StorageError::Other(format!("invalid archive key: {e}")))
    }

    pub async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        self.store.put(&Self::path(key)?, PutPayload::from(bytes)).await.map_err(obj_err)?;
        Ok(())
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let got = self.store.get(&Self::path(key)?).await.map_err(obj_err)?;
        Ok(got.bytes().await.map_err(obj_err)?.to_vec())
    }

    /// Deleting a missing object is not an error.
    pub async fn delete(&self, key: &str) -> Result<()> {
        match self.store.delete(&Self::path(key)?).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(obj_err(e)),
        }
    }
}

/// A key component: IDs are used verbatim when they are plain, otherwise hex-encoded behind a
/// `~` (which IDs never contain), so no ID can form `..` or a separator.
fn component(id: &str) -> String {
    let plain = !id.is_empty()
        && id != "."
        && id != ".."
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if plain { id.to_owned() } else { format!("~{}", hex::encode(id.as_bytes())) }
}

fn tenant_prefix(kind: &str, s: &Scope) -> String {
    format!(
        "{kind}/{LOGS}/{}/{}/{}/",
        component(s.tenant_id.as_str()),
        component(s.project_id.as_str()),
        component(s.environment_id.as_str()),
    )
}

fn scope_prefix(kind: &str, s: &Scope, service_id: &str) -> String {
    format!("{}{}", tenant_prefix(kind, s), component(service_id))
}

/// Segment keys carry a content hash, so a re-archived window never overwrites an object that a
/// reader may be fetching.
pub fn segment_key(s: &Scope, service_id: &str, window_start_ns: i64, sha256: &str) -> String {
    format!("{}/{window_start_ns}-{}.chseg", scope_prefix("segments", s, service_id), &sha256[..sha256.len().min(16)])
}

/// Dictionary keys carry a content hash too: an archiver uploads the object before it claims the
/// version in the catalog, and a losing concurrent upload can never replace the winner's bytes.
pub fn dict_key(s: &Scope, service_id: &str, version: u32, sha256: &str) -> String {
    format!("{}/{version}-{}.chdict", scope_prefix("dicts", s, service_id), &sha256[..sha256.len().min(16)])
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

// ---------------------------------------------------------------- catalog

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SegmentRow {
    pub tenant_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub service_id: String,
    pub window_start_ns: i64,
    pub window_end_ns: i64,
    pub min_time_ns: i64,
    pub max_time_ns: i64,
    pub rows: i64,
    pub source_bytes: i64,
    pub stored_bytes: i64,
    pub templates: i32,
    pub dict_version: i32,
    pub object_key: String,
    pub sha256: String,
    /// Object key of the segment's dictionary (catalog join; ignored on insert).
    pub dict_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct WindowRow {
    pub window_start_ns: i64,
    pub window_end_ns: i64,
    pub rows: i64,
}

const SEGMENT_COLUMNS: &str = "tenant_id, project_id, environment_id, service_id, window_start_ns, window_end_ns, \
     min_time_ns, max_time_ns, rows, source_bytes, stored_bytes, templates, dict_version, object_key, sha256";
const SEGMENT_SELECT: &str = "s.tenant_id, s.project_id, s.environment_id, s.service_id, s.window_start_ns, s.window_end_ns, \
     s.min_time_ns, s.max_time_ns, s.rows, s.source_bytes, s.stored_bytes, s.templates, s.dict_version, s.object_key, s.sha256";

/// u64 template IDs are stored bit-for-bit in BIGINT columns.
pub fn template_id_to_db(id: u64) -> i64 {
    i64::from_le_bytes(id.to_le_bytes())
}

/// Catalog condition: the segment's bloom filter may hold the trace whose probe hashes are bound
/// at `$h1`/`$h2` (both NULL: no trace filter). Mirrors `karbird_archive::index`.
fn bloom_sql(h1: usize, h2: usize) -> String {
    let probes: Vec<String> = (0..BLOOM_HASHES)
        .map(|i| {
            format!("get_bit(s.trace_bloom, ((${h1} + {i} * ${h2}) % (octet_length(s.trace_bloom) * 8))::int) = 1")
        })
        .collect();
    format!("(${h1}::bigint IS NULL OR s.trace_bloom IS NULL OR ({}))", probes.join(" AND "))
}

impl PgStore {
    /// Inserts or replaces the segment of `(scope, service, window)` with its search index.
    /// Returns the object key it replaced when that differs, so the caller can delete the old object.
    pub async fn upsert_archive_segment(&self, seg: &SegmentRow, index: &SegmentIndex) -> Result<Option<String>> {
        let old: Option<String> = sqlx::query_scalar(
            "SELECT object_key FROM archive_segments
             WHERE tenant_id = $1 AND project_id = $2 AND environment_id = $3 AND signal = $4 AND service_id = $5 AND window_start_ns = $6",
        )
        .bind(&seg.tenant_id)
        .bind(&seg.project_id)
        .bind(&seg.environment_id)
        .bind(LOGS)
        .bind(&seg.service_id)
        .bind(seg.window_start_ns)
        .fetch_optional(self.pool())
        .await?;
        sqlx::query(&format!(
            "INSERT INTO archive_segments (signal, {SEGMENT_COLUMNS}, template_ids, max_severity, trace_bloom)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
             ON CONFLICT (tenant_id, project_id, environment_id, signal, service_id, window_start_ns) DO UPDATE SET
               window_end_ns = EXCLUDED.window_end_ns, min_time_ns = EXCLUDED.min_time_ns, max_time_ns = EXCLUDED.max_time_ns,
               rows = EXCLUDED.rows, source_bytes = EXCLUDED.source_bytes, stored_bytes = EXCLUDED.stored_bytes,
               templates = EXCLUDED.templates, dict_version = EXCLUDED.dict_version, object_key = EXCLUDED.object_key,
               sha256 = EXCLUDED.sha256, template_ids = EXCLUDED.template_ids, max_severity = EXCLUDED.max_severity,
               trace_bloom = EXCLUDED.trace_bloom, created_at = now()"
        ))
        .bind(LOGS)
        .bind(&seg.tenant_id)
        .bind(&seg.project_id)
        .bind(&seg.environment_id)
        .bind(&seg.service_id)
        .bind(seg.window_start_ns)
        .bind(seg.window_end_ns)
        .bind(seg.min_time_ns)
        .bind(seg.max_time_ns)
        .bind(seg.rows)
        .bind(seg.source_bytes)
        .bind(seg.stored_bytes)
        .bind(seg.templates)
        .bind(seg.dict_version)
        .bind(&seg.object_key)
        .bind(&seg.sha256)
        .bind(index.template_ids.as_ref().map(|ids| ids.iter().copied().map(template_id_to_db).collect::<Vec<_>>()))
        .bind(i16::from(index.max_severity))
        .bind(&index.trace_bloom)
        .execute(self.pool())
        .await?;
        Ok(old.filter(|k| *k != seg.object_key))
    }

    /// Segments of `scope` that may hold rows matching `q`, newest window first. The catalog
    /// index skips segments that cannot match the service, severity, trace or template filter,
    /// and the cursor skips segments that only hold rows newer than it.
    pub async fn archive_segments(&self, s: &Scope, q: &LogQuery, limit: i64) -> Result<Vec<SegmentRow>> {
        let probe = q.trace_id.as_deref().map(bloom_probe);
        Ok(sqlx::query_as::<_, SegmentRow>(&format!(
            "SELECT {SEGMENT_SELECT}, d.object_key AS dict_key FROM archive_segments s
             LEFT JOIN archive_dictionaries d ON d.tenant_id = s.tenant_id AND d.project_id = s.project_id
               AND d.environment_id = s.environment_id AND d.signal = s.signal AND d.service_id = s.service_id
               AND d.version = s.dict_version
             WHERE s.tenant_id = $1 AND s.project_id = $2 AND s.environment_id = $3 AND s.signal = $4
               AND s.max_time_ns >= $5 AND s.min_time_ns < $6 AND ($7::text IS NULL OR s.service_id = $7)
               AND ($8::bigint IS NULL OR s.min_time_ns <= $8)
               AND ($9::smallint IS NULL OR s.max_severity IS NULL OR s.max_severity >= $9)
               AND ($10::bigint IS NULL OR s.template_ids IS NULL OR $10 = ANY(s.template_ids))
               AND {}
             ORDER BY s.window_start_ns DESC, s.service_id
             LIMIT $13",
            bloom_sql(11, 12)
        ))
        .bind(s.tenant_id.as_str())
        .bind(s.project_id.as_str())
        .bind(s.environment_id.as_str())
        .bind(LOGS)
        .bind(q.from_ns)
        .bind(q.to_ns)
        .bind(q.service_id.as_deref())
        .bind(q.before.as_ref().map(|(t, _)| *t))
        .bind(q.min_severity.map(i16::from))
        .bind(q.template_id.map(template_id_to_db))
        .bind(probe.map(|(h1, _)| i64::from(h1)))
        .bind(probe.map(|(_, h2)| i64::from(h2)))
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    /// Oldest archived log for a scope, or `None` when no archived log is catalogued.
    pub async fn earliest_archive_log_time(&self, s: &Scope) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT min(min_time_ns) FROM archive_segments
             WHERE tenant_id = $1 AND project_id = $2 AND environment_id = $3
               AND signal = 'logs' AND rows > 0",
        )
        .bind(s.tenant_id.as_str())
        .bind(s.project_id.as_str())
        .bind(s.environment_id.as_str())
        .fetch_one(self.pool())
        .await?)
    }

    /// Newest dictionary of a service: (version, object key).
    pub async fn latest_archive_dictionary(&self, s: &Scope, service_id: &str) -> Result<Option<(u32, String)>> {
        let row: Option<(i32, String)> = sqlx::query_as(
            "SELECT version, object_key FROM archive_dictionaries
             WHERE tenant_id = $1 AND project_id = $2 AND environment_id = $3 AND signal = $4 AND service_id = $5
             ORDER BY version DESC LIMIT 1",
        )
        .bind(s.tenant_id.as_str())
        .bind(s.project_id.as_str())
        .bind(s.environment_id.as_str())
        .bind(LOGS)
        .bind(service_id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.and_then(|(v, k)| u32::try_from(v).ok().map(|v| (v, k))))
    }

    /// Records a dictionary version; `false` when another archiver already wrote that version.
    pub async fn insert_archive_dictionary(
        &self,
        s: &Scope,
        service_id: &str,
        dict: &Dictionary,
        object_key: &str,
    ) -> Result<bool> {
        let version = i32::try_from(dict.version()).map_err(|_| StorageError::Other("dictionary version".into()))?;
        let done = sqlx::query(
            "INSERT INTO archive_dictionaries (tenant_id, project_id, environment_id, signal, service_id, version, object_key, entries, bytes)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT DO NOTHING",
        )
        .bind(s.tenant_id.as_str())
        .bind(s.project_id.as_str())
        .bind(s.environment_id.as_str())
        .bind(LOGS)
        .bind(service_id)
        .bind(version)
        .bind(object_key)
        .bind(i32::try_from(dict.entries().len()).unwrap_or(i32::MAX))
        .bind(i32::try_from(dict.as_bytes().len()).unwrap_or(i32::MAX))
        .execute(self.pool())
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// End of the newest window the archiver has finished (archived or lost).
    pub async fn last_archive_window_end(&self) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar("SELECT max(window_end_ns) FROM archive_windows WHERE signal = $1")
            .bind(LOGS)
            .fetch_one(self.pool())
            .await?)
    }

    pub async fn record_archive_window(
        &self,
        start: i64,
        end: i64,
        archived: bool,
        rows: i64,
        segments: i32,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO archive_windows (signal, window_start_ns, window_end_ns, status, rows, segments)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (signal, window_start_ns) DO UPDATE SET
               window_end_ns = EXCLUDED.window_end_ns, status = EXCLUDED.status, rows = EXCLUDED.rows,
               segments = EXCLUDED.segments, updated_at = now()",
        )
        .bind(LOGS)
        .bind(start)
        .bind(end)
        .bind(if archived { "archived" } else { "lost" })
        .bind(rows)
        .bind(segments)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Archived windows that ended before `ended_before` and were not re-checked for late data.
    pub async fn archive_windows_to_recheck(&self, ended_before: i64, limit: i64) -> Result<Vec<WindowRow>> {
        Ok(sqlx::query_as::<_, WindowRow>(
            "SELECT window_start_ns, window_end_ns, rows FROM archive_windows
             WHERE signal = $1 AND status = 'archived' AND rechecked_at IS NULL AND window_end_ns < $2
             ORDER BY window_start_ns LIMIT $3",
        )
        .bind(LOGS)
        .bind(ended_before)
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn mark_archive_window_rechecked(&self, start: i64, rows: i64, segments: Option<i32>) -> Result<()> {
        sqlx::query(
            "UPDATE archive_windows SET rechecked_at = now(), rows = $3, segments = COALESCE($4, segments)
             WHERE signal = $1 AND window_start_ns = $2",
        )
        .bind(LOGS)
        .bind(start)
        .bind(rows)
        .bind(segments)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Segments past the archive retention, or of projects that were deleted.
    pub async fn expired_archive_segments(&self, ended_before: i64, limit: i64) -> Result<Vec<SegmentRow>> {
        Ok(sqlx::query_as::<_, SegmentRow>(&format!(
            "SELECT {SEGMENT_SELECT}, NULL::text AS dict_key FROM archive_segments s
             WHERE s.window_end_ns < $1 OR NOT EXISTS (SELECT 1 FROM projects p WHERE p.id = s.project_id)
             ORDER BY s.window_end_ns LIMIT $2"
        ))
        .bind(ended_before)
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn delete_archive_segment(&self, seg: &SegmentRow) -> Result<()> {
        sqlx::query(
            "DELETE FROM archive_segments
             WHERE tenant_id = $1 AND project_id = $2 AND environment_id = $3 AND signal = $4 AND service_id = $5
               AND window_start_ns = $6 AND object_key = $7",
        )
        .bind(&seg.tenant_id)
        .bind(&seg.project_id)
        .bind(&seg.environment_id)
        .bind(LOGS)
        .bind(&seg.service_id)
        .bind(seg.window_start_ns)
        .bind(&seg.object_key)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Dictionaries of deleted projects: (object key, scope columns) to remove.
    pub async fn orphaned_archive_dictionaries(
        &self,
        limit: i64,
    ) -> Result<Vec<(String, String, String, String, i32)>> {
        Ok(sqlx::query_as(
            "SELECT d.object_key, d.project_id, d.environment_id, d.service_id, d.version FROM archive_dictionaries d
             WHERE NOT EXISTS (SELECT 1 FROM projects p WHERE p.id = d.project_id) LIMIT $1",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn delete_archive_dictionary(&self, object_key: &str) -> Result<()> {
        sqlx::query("DELETE FROM archive_dictionaries WHERE object_key = $1")
            .bind(object_key)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------- reader

/// Whether an archived row passes the same filters the ClickHouse log query applies.
pub fn log_matches(q: &LogQuery, row: &LogRow) -> bool {
    row.time >= q.from_ns
        && row.time < q.to_ns
        && q.service_id.as_ref().is_none_or(|s| *s == row.service_id)
        && q.min_severity.is_none_or(|m| row.severity_number >= m)
        && q.trace_id.as_ref().is_none_or(|t| *t == row.trace_id)
        // ASCII-only folding, like ClickHouse's `positionCaseInsensitive`.
        && q.text.as_ref().is_none_or(|t| row.body.to_ascii_lowercase().contains(&t.to_ascii_lowercase()))
        && q.template_id.is_none_or(|t| t == row.template_id)
        && q.before.as_ref().is_none_or(|(t, id)| (row.time, row.log_id.as_str()) < (*t, id.as_str()))
}

pub fn log_row(seg: &SegmentRow, r: LogRecord) -> LogRow {
    let template_id = karbird_archive::template_id(&karbird_archive::parse(&r.body).template);
    LogRow {
        tenant_id: seg.tenant_id.clone(),
        project_id: seg.project_id.clone(),
        environment_id: seg.environment_id.clone(),
        service_id: seg.service_id.clone(),
        service_name: r.service_name,
        service_instance_id: r.service_instance_id,
        log_id: r.log_id,
        time: r.time,
        observed_time: r.observed_time,
        severity_number: r.severity_number,
        severity_text: r.severity_text,
        body: r.body,
        template_id,
        trace_id: r.trace_id,
        span_id: r.span_id,
        attributes: r.attributes,
        resource_attributes: r.resource_attributes,
        batch_id: r.batch_id,
        ingested_at: r.ingested_at,
    }
}

/// Reads archived logs for the API. Only segments of the caller's scope are listed, and every
/// object is checked against the catalog's SHA-256 before it is decoded.
#[derive(Debug, Clone)]
pub struct ArchiveReader {
    objects: ArchiveObjects,
    dicts: Arc<Mutex<HashMap<String, Arc<Dictionary>>>>,
}

impl ArchiveReader {
    pub fn new(objects: ArchiveObjects) -> Self {
        Self { objects, dicts: Arc::new(Mutex::new(HashMap::new())) }
    }

    async fn dictionary(&self, s: &Scope, key: &str, version: u32) -> Result<Arc<Dictionary>> {
        if !key.starts_with(&tenant_prefix("dicts", s)) {
            return Err(StorageError::Other("archive dictionary outside the caller's scope".into()));
        }
        if let Some(d) = self.dicts.lock().await.get(key) {
            return Ok(d.clone());
        }
        let bytes = self.objects.get(key).await?;
        let dict = Arc::new(Dictionary::from_bytes(version, &bytes).map_err(|e| StorageError::Other(e.to_string()))?);
        let mut cache = self.dicts.lock().await;
        if cache.len() >= DICT_CACHE_ENTRIES {
            cache.clear();
        }
        cache.insert(key.to_owned(), dict.clone());
        Ok(dict)
    }

    async fn segment_logs(&self, s: &Scope, seg: &SegmentRow) -> Result<Vec<LogRecord>> {
        let bytes = self.objects.get(&seg.object_key).await?;
        if sha256_hex(&bytes) != seg.sha256 {
            return Err(StorageError::Other(format!("archive segment {} failed its checksum", seg.object_key)));
        }
        let dict = match u32::try_from(seg.dict_version).unwrap_or(0) {
            0 => None,
            v => {
                let key = seg.dict_key.as_deref().ok_or_else(|| {
                    StorageError::Other(format!("archive segment {} has no catalogued dictionary", seg.object_key))
                })?;
                Some(self.dictionary(s, key, v).await?)
            }
        };
        tokio::task::spawn_blocking(move || karbird_archive::decode(&bytes, dict.as_deref()))
            .await
            .map_err(|e| StorageError::Other(e.to_string()))?
            .map_err(|e| StorageError::Other(e.to_string()))
    }

    /// Up to `q.limit` matching rows from `segments` (newest window first, as listed by
    /// [`PgStore::archive_segments`]), newest first. Stops opening segments once the newer
    /// windows already filled the page.
    pub async fn logs_from_segments(&self, s: &Scope, segments: &[SegmentRow], q: &LogQuery) -> Result<Vec<LogRow>> {
        Ok(self.page_from_segments(s, segments, q, false).await?.rows)
    }

    /// Like [`Self::logs_from_segments`]. `capped`: the listing hit [`MAX_SEGMENTS_PER_QUERY`],
    /// so older segments exist that were not listed. Its last window may then be listed only in
    /// part; it is left out (unless it is the only one) and the page says where to resume.
    pub async fn page_from_segments(
        &self,
        s: &Scope,
        segments: &[SegmentRow],
        q: &LogQuery,
        capped: bool,
    ) -> Result<ArchivePage> {
        let (segments, resume) = match (capped, segments.last()) {
            (true, Some(last)) => {
                let complete = segments.iter().take_while(|g| g.window_start_ns != last.window_start_ns).count();
                if complete == 0 {
                    (segments, Some(last.window_start_ns))
                } else {
                    (&segments[..complete], segments.get(complete - 1).map(|g| g.window_start_ns))
                }
            }
            _ => (segments, None),
        };
        let rows = self.scan(s, segments, q).await?;
        let resume_before = resume.filter(|_| rows.len() < q.limit as usize);
        Ok(ArchivePage { rows, resume_before })
    }

    async fn scan(&self, s: &Scope, segments: &[SegmentRow], q: &LogQuery) -> Result<Vec<LogRow>> {
        let limit = q.limit as usize;
        let mut out: Vec<LogRow> = Vec::new();
        let mut i = 0;
        while i < segments.len() {
            let window = segments[i].window_start_ns;
            while let Some(seg) = segments.get(i).filter(|g| g.window_start_ns == window) {
                let foreign = seg.tenant_id != s.tenant_id.as_str()
                    || seg.project_id != s.project_id.as_str()
                    || seg.environment_id != s.environment_id.as_str();
                if !foreign {
                    for r in self.segment_logs(s, seg).await? {
                        let row = log_row(seg, r);
                        if log_matches(q, &row) {
                            out.push(row);
                        }
                    }
                }
                i += 1;
            }
            if out.len() >= limit {
                break;
            }
        }
        out.sort_by(|a, b| (b.time, &b.log_id).cmp(&(a.time, &a.log_id)));
        out.truncate(limit);
        Ok(out)
    }

    pub async fn logs(&self, pg: &PgStore, s: &Scope, q: &LogQuery) -> Result<ArchivePage> {
        let mut segments = pg.archive_segments(s, q, MAX_SEGMENTS_PER_QUERY + 1).await?;
        let capped = segments.len() > MAX_SEGMENTS_PER_QUERY as usize;
        segments.truncate(MAX_SEGMENTS_PER_QUERY as usize);
        self.page_from_segments(s, &segments, q, capped).await
    }
}

/// One read of the archive.
#[derive(Debug, Clone, Default)]
pub struct ArchivePage {
    /// Newest first, at most the query's limit.
    pub rows: Vec<LogRow>,
    /// Set when the segment cap stopped the read before the page filled: every match at or
    /// after this time was returned, older ones need another read with `before = (this, "")`.
    pub resume_before: Option<i64>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use karbird_domain::{EnvironmentId, ProjectId, TenantId};

    fn scope(t: &str) -> Scope {
        Scope {
            tenant_id: TenantId::parse(t).unwrap(),
            project_id: ProjectId::parse("p1").unwrap(),
            environment_id: EnvironmentId::parse("e1").unwrap(),
        }
    }

    const T0: i64 = 1_790_000_000_000_000_000;

    fn record(i: i64, body: &str, severity: u8) -> LogRecord {
        LogRecord {
            service_name: "orders".into(),
            log_id: format!("{i:032x}"),
            time: T0 + i * 1_000,
            observed_time: T0 + i * 1_000,
            severity_number: severity,
            severity_text: "INFO".into(),
            body: body.into(),
            trace_id: if i % 2 == 0 { "ab".repeat(16) } else { String::new() },
            ingested_at: T0,
            ..LogRecord::default()
        }
    }

    async fn store_segment(objects: &ArchiveObjects, s: &Scope, window: i64, records: &[LogRecord]) -> SegmentRow {
        let enc = karbird_archive::encode(records, None, 3).unwrap();
        let sha = sha256_hex(&enc.bytes);
        let key = segment_key(s, "svc_a", window, &sha);
        objects.put(&key, enc.bytes).await.unwrap();
        SegmentRow {
            tenant_id: s.tenant_id.to_string(),
            project_id: s.project_id.to_string(),
            environment_id: s.environment_id.to_string(),
            service_id: "svc_a".into(),
            window_start_ns: window,
            window_end_ns: window + 1_000_000,
            min_time_ns: enc.stats.min_time,
            max_time_ns: enc.stats.max_time,
            rows: enc.stats.rows as i64,
            source_bytes: enc.stats.source_bytes as i64,
            stored_bytes: enc.stats.stored_bytes as i64,
            templates: enc.stats.templates as i32,
            dict_version: 0,
            object_key: key,
            sha256: sha,
            dict_key: None,
        }
    }

    fn query(limit: u32) -> LogQuery {
        LogQuery { from_ns: T0, to_ns: T0 + 1_000_000_000, limit, ..LogQuery::default() }
    }

    #[test]
    fn keys_are_scoped_and_cannot_escape_their_prefix() {
        let s = scope("t1");
        let k = segment_key(&s, "svc_a", 42, &"f".repeat(64));
        assert_eq!(k, format!("segments/logs/t1/p1/e1/svc_a/42-{}.chseg", "f".repeat(16)));
        assert_eq!(
            dict_key(&s, "svc_a", 3, &"a".repeat(64)),
            format!("dicts/logs/t1/p1/e1/svc_a/3-{}.chdict", "a".repeat(16))
        );
        assert_eq!(component(".."), "~2e2e");
        assert_eq!(component("a:b"), "~613a62", "characters outside the plain set are hex-encoded");
        assert_eq!(component(""), "~");
        assert_eq!(component("svc_ab-1.x"), "svc_ab-1.x");
        assert!(ArchiveObjects::path(&dict_key(&scope(".."), "..", 1, "..")).is_ok());
        assert_ne!(segment_key(&scope("t1"), "s", 1, "aa"), segment_key(&scope("t2"), "s", 1, "aa"));
    }

    #[test]
    fn debug_output_hides_credentials() {
        let cfg = ObjectConfig {
            endpoint: "http://127.0.0.1:19000".into(),
            bucket: "b".into(),
            region: "us-east-1".into(),
            access_key: "AKIA-visible-nowhere".into(),
            secret_key: "super-secret-key".into(),
        };
        let out = format!("{cfg:?}");
        assert!(!out.contains("super-secret-key") && !out.contains("AKIA"), "{out}");
    }

    #[test]
    fn filters_match_the_clickhouse_query() {
        let seg = SegmentRow {
            tenant_id: "t".into(),
            project_id: "p".into(),
            environment_id: "e".into(),
            service_id: "svc_a".into(),
            window_start_ns: 0,
            window_end_ns: 1,
            min_time_ns: 0,
            max_time_ns: 0,
            rows: 0,
            source_bytes: 0,
            stored_bytes: 0,
            templates: 0,
            dict_version: 0,
            object_key: String::new(),
            sha256: String::new(),
            dict_key: None,
        };
        let row = log_row(&seg, record(5, "Payment FAILED for order 7", 17));
        let q = query(10);
        assert!(log_matches(&q, &row));
        assert!(log_matches(&LogQuery { text: Some("payment failed".into()), ..q.clone() }, &row), "case-insensitive");
        assert!(!log_matches(&LogQuery { text: Some("refund".into()), ..q.clone() }, &row));
        let unicode = log_row(&seg, record(6, "Größe ÜBER limit", 9));
        assert!(
            !log_matches(&LogQuery { text: Some("größe über LIMIT".into()), ..q.clone() }, &unicode),
            "only ASCII folds, as in ClickHouse"
        );
        assert!(log_matches(&LogQuery { text: Some("GRößE ÜBER LIMIT".into()), ..q.clone() }, &unicode));
        assert!(!log_matches(&LogQuery { min_severity: Some(21), ..q.clone() }, &row));
        assert!(!log_matches(&LogQuery { service_id: Some("svc_b".into()), ..q.clone() }, &row));
        assert!(!log_matches(&LogQuery { trace_id: Some("cd".repeat(16)), ..q.clone() }, &row));
        assert!(!log_matches(&LogQuery { to_ns: row.time, ..q.clone() }, &row), "to is exclusive");
        assert!(
            !log_matches(&LogQuery { before: Some((row.time, row.log_id.clone())), ..q.clone() }, &row),
            "cursor is exclusive"
        );
        assert!(log_matches(&LogQuery { before: Some((row.time + 1, String::new())), ..q.clone() }, &row));
        assert_eq!(row.template_id, karbird_archive::template_id(&karbird_archive::parse(&row.body).template));
        assert!(log_matches(&LogQuery { template_id: Some(row.template_id), ..q.clone() }, &row));
        assert!(!log_matches(&LogQuery { template_id: Some(row.template_id ^ 1), ..q }, &row));
    }

    #[test]
    fn template_ids_round_trip_through_bigint() {
        for id in [0, 1, u64::MAX, 1 << 63, 0x0123_4567_89ab_cdef] {
            let db = template_id_to_db(id);
            assert_eq!(u64::from_le_bytes(db.to_le_bytes()), id);
        }
        assert_eq!(template_id_to_db(u64::MAX), -1, "bit-for-bit, not saturated");
    }

    #[test]
    fn bloom_condition_checks_every_probe_and_binds_only_numbers() {
        let sql = bloom_sql(11, 12);
        assert_eq!(sql.matches("get_bit(").count(), BLOOM_HASHES as usize);
        assert!(sql.starts_with("($11::bigint IS NULL OR s.trace_bloom IS NULL OR ("), "{sql}");
        for i in 0..BLOOM_HASHES {
            assert!(sql.contains(&format!("(($11 + {i} * $12) % (octet_length(s.trace_bloom) * 8))::int")), "{sql}");
        }
        assert!(!sql.contains('\''), "no literals: {sql}");
    }

    #[tokio::test]
    async fn capped_listings_resume_at_the_last_complete_window() {
        let objects = ArchiveObjects::in_memory();
        let reader = ArchiveReader::new(objects.clone());
        let s = scope("t1");
        let w = |k: i64| T0 + k * 100_000;
        let seg_a = store_segment(&objects, &s, w(2), &[record(250, "order a created", 9)]).await;
        let seg_b = store_segment(&objects, &s, w(1), &[record(150, "order b created", 9)]).await;
        let mut seg_b2 = seg_b.clone();
        seg_b2.service_id = "svc_b".into();
        let segments = vec![seg_a.clone(), seg_b.clone(), seg_b2];

        let page = reader.page_from_segments(&s, &segments, &query(10), true).await.unwrap();
        assert_eq!(page.rows.len(), 1, "the possibly partial last window is left for the next read");
        assert_eq!(page.resume_before, Some(w(2)), "every match at or after the resume time was returned");

        let next = LogQuery { before: Some((w(2), String::new())), ..query(10) };
        assert!(log_matches(&next, &log_row(&seg_b, record(150, "x", 9))), "the resume cursor reaches older rows");
        assert!(!log_matches(&next, &log_row(&seg_a, record(250, "x", 9))));

        let full = reader.page_from_segments(&s, &segments, &query(1), true).await.unwrap();
        assert_eq!((full.rows.len(), full.resume_before), (1, None), "a full page pages with its own cursor");

        let only = reader.page_from_segments(&s, std::slice::from_ref(&seg_b), &query(10), true).await.unwrap();
        assert_eq!((only.rows.len(), only.resume_before), (1, Some(w(1))), "a single window is read, not dropped");

        let all = reader.page_from_segments(&s, &segments[..2], &query(10), false).await.unwrap();
        assert_eq!((all.rows.len(), all.resume_before), (2, None), "not capped: nothing to resume");
    }

    #[tokio::test]
    async fn reader_returns_newest_first_pages_with_the_cursor_and_verifies_checksums() {
        let objects = ArchiveObjects::in_memory();
        let reader = ArchiveReader::new(objects.clone());
        let s = scope("t1");
        let old: Vec<_> = (0..5).map(|i| record(i, &format!("order {i} created"), 9)).collect();
        let new: Vec<_> = (5..10).map(|i| record(i, &format!("order {i} created"), 9)).collect();
        let seg_new = store_segment(&objects, &s, T0 + 5_000, &new).await;
        let seg_old = store_segment(&objects, &s, T0, &old).await;
        let segments = vec![seg_new.clone(), seg_old.clone()];

        let page = reader.logs_from_segments(&s, &segments, &query(3)).await.unwrap();
        let times: Vec<i64> = page.iter().map(|r| (r.time - T0) / 1_000).collect();
        assert_eq!(times, [9, 8, 7]);
        assert!(page.iter().all(|r| r.tenant_id == "t1" && r.service_id == "svc_a"));

        let last = page.last().unwrap();
        let next = LogQuery { before: Some((last.time, last.log_id.clone())), ..query(3) };
        let page2 = reader.logs_from_segments(&s, &segments, &next).await.unwrap();
        assert_eq!(page2.iter().map(|r| (r.time - T0) / 1_000).collect::<Vec<_>>(), [6, 5, 4]);

        let errors = LogQuery { trace_id: Some("ab".repeat(16)), ..query(100) };
        assert_eq!(
            reader.logs_from_segments(&s, &segments, &errors).await.unwrap().len(),
            5,
            "even rows carry the trace"
        );

        let mut tampered = seg_old.clone();
        tampered.sha256 = "0".repeat(64);
        let err = reader.logs_from_segments(&s, &[tampered], &query(10)).await.unwrap_err();
        assert!(err.to_string().contains("checksum"), "{err}");
    }

    #[tokio::test]
    async fn reader_ignores_segments_of_another_scope() {
        let objects = ArchiveObjects::in_memory();
        let reader = ArchiveReader::new(objects.clone());
        let mine = scope("t1");
        let other = scope("t2");
        let foreign = store_segment(&objects, &other, T0, &[record(1, "secret of t2", 9)]).await;
        let got = reader.logs_from_segments(&mine, &[foreign], &query(10)).await.unwrap();
        assert!(got.is_empty(), "a mis-listed segment of another tenant is never decoded into the response");
    }

    #[tokio::test]
    async fn reader_uses_the_segment_dictionary() {
        let objects = ArchiveObjects::in_memory();
        let reader = ArchiveReader::new(objects.clone());
        let s = scope("t1");
        let records: Vec<_> =
            (0..20).map(|i| record(i, &format!("order {i} created for user u_{}", i % 3), 9)).collect();
        let dict = Dictionary::new(
            4,
            vec![format!(
                "order {} created for user {}",
                karbird_archive::template::INT_VAR,
                karbird_archive::template::STR_VAR
            )],
        );
        let dkey = dict_key(&s, "svc_a", 4, &sha256_hex(dict.as_bytes()));
        objects.put(&dkey, dict.as_bytes().to_vec()).await.unwrap();
        let enc = karbird_archive::encode(&records, Some(&dict), 3).unwrap();
        let mut seg = store_segment(&objects, &s, T0, &[]).await;
        seg.sha256 = sha256_hex(&enc.bytes);
        objects.put(&seg.object_key, enc.bytes).await.unwrap();
        seg.dict_version = 4;

        let err = reader.logs_from_segments(&s, &[seg.clone()], &query(100)).await.unwrap_err();
        assert!(err.to_string().contains("no catalogued dictionary"), "{err}");
        let foreign = Scope { tenant_id: TenantId::parse("t2").unwrap(), ..s.clone() };
        let stolen = SegmentRow { dict_key: Some(dict_key(&foreign, "svc_a", 4, "aa")), ..seg.clone() };
        let err = reader.logs_from_segments(&s, &[stolen], &query(100)).await.unwrap_err();
        assert!(err.to_string().contains("outside the caller's scope"), "{err}");

        seg.dict_key = Some(dkey.clone());
        let got = reader.logs_from_segments(&s, &[seg.clone()], &query(100)).await.unwrap();
        assert_eq!(got.len(), 20);
        objects.delete(&dkey).await.unwrap();
        assert_eq!(reader.logs_from_segments(&s, &[seg], &query(100)).await.unwrap().len(), 20, "cached");
        objects.delete("never/existed").await.unwrap();
    }
}
