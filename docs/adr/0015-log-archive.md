# ADR-0015: Log archive: templates, columnar segments, per-service dictionaries, separate archiver

## Decision
- **Templates at ingest time.** The processor splits every log body into a template and its
  variables (`karbird-archive::template`): words that contain a digit, or long hex words, are
  variables; integers are stored as numbers. `template_id` (first 8 bytes of SHA-256 of the
  template) is written to ClickHouse and returned by the API, so lines of one kind can be grouped.
  The split is lossless: `render(parse(body)) == body` for every input (property test).
- **ClickHouse stays the hot store** (TTL from `KARBIRD_LOGS_RETENTION_DAYS`, 14 days by
  default, ADR-0016) and gets column codecs (Delta/T64 + ZSTD) on
  non-key columns (`0002_log_templates_and_codecs.sql`).
- **A separate process, `karbird-archiver`**, copies closed one-hour windows (UTC) per
  service out of ClickHouse into S3-compatible object storage. It only reads ClickHouse; the
  table TTL deletes. It is not in the ingest path, so the ACK semantics are unchanged.
- **Segment format `CHSEG`** (`karbird-archive::segment`): rows are sorted by template, time
  and ID; every field is its own column (delta/zigzag varints for times, an interned string table,
  attribute and resource sets referenced by index, hex IDs as raw bytes, template variables typed),
  each column compressed with zstd (level 19).
- **Per-service zstd dictionary** (`CHDICT`): built from the templates and repeated strings of
  the segments archived so far, versioned, capped at 112 KiB, oldest entries dropped first. A
  segment records the dictionary version it needs. Small segments of a service that has been
  archived before therefore compress with everything already learned about that service:
  efficiency grows with the amount of logs a service has produced, not only with the size of
  one segment.
- **Catalog in PostgreSQL** (`archive_segments`, `archive_dictionaries`, `archive_windows`):
  object key, SHA-256, row count, time range and dictionary version per segment. Object keys are
  scoped by tenant/project/environment/service and carry a content hash, so a re-archived window
  never overwrites an object a reader may be fetching.
- **Reading through the API.** `GET /logs?source=auto|hot|archive` (default `auto`): ranges older
  than the hot window (one day less than the log retention by default, always below it) are read from the archive, newer ones
  from ClickHouse; a range spanning both is served newest first from ClickHouse, then the archive.
  The response says which (`source: hot|archive|mixed`). The reader lists segments only for the
  caller's scope, verifies each object's SHA-256 against the catalog and decodes off the async
  runtime. Without archive configuration `auto` means ClickHouse only and `archive` answers 503.
- **Range and work bounds.** Log reads that may use the archive (`auto` or `archive` with an
  archive configured) accept ranges up to 31 days; ClickHouse-only reads and every other route
  keep 7 days. One request opens at most 400 segments. When that cap stops a read before the page
  is full, the response is `truncated` with a `next_cursor` at the last fully read window, so
  paging continues without gaps or repeats (more than 400 services in one window of one
  environment would be the exception: that window is read only in part).
- **Search index in the catalog** (migration 0008), filled by the archiver per segment: the
  sorted template IDs, the highest severity and a bloom filter over trace IDs (10 bits per trace,
  7 probes, 64 bits to 128 KiB; layout in `karbird-archive/src/index.rs`). PostgreSQL applies
  them while listing segments (`get_bit` for the bloom probes), so segments that cannot match the
  `template_id`, `min_severity` or `trace_id` filter are never fetched and do not count against
  the 400-segment cap. Segments without an index (archived earlier) are always opened. Text
  search (`q`) cannot use it and still filters while decoding.
- **In the UI.** The logs panel has a source switch (Auto / Hot / Archive), labels pages read
  from the archive ("From archive", "Hot + archive"), pages further with "Load older", and filters
  lines of one kind ("like", `template_id`). The timeline reads archived logs for the part of its
  range before the hot window (`logs_source` in `/timeline`) and says that traces are not
  archived; `/timeline`, `/topology` and `/services` add a completeness note for such ranges.
- **Local object storage: SeaweedFS** (`chrislusf/seaweedfs:3.97`) in Docker Compose; production
  uses any S3-compatible service. MinIO was the first choice, but its images are no longer
  published on Docker Hub (and `quay.io` requires credentials). Only the S3 API is used
  (`object_store` crate), so the store is replaceable.

## Guarantees
- **No silent loss.** A window is recorded as archived only after every service's segment is
  encoded, decoded back and compared with the source rows, stored, read back with a matching
  SHA-256 and catalogued. A crash means the window is archived again. Windows that left
  ClickHouse before they could be archived (archiver down longer than the TTL) are recorded as
  `lost` and counted (`karbird_archiver_windows_total{result="lost"}`), never skipped quietly.
- **Late data.** Each window is counted again 24 hours after it closed; when ClickHouse has more
  rows than were archived, the window is archived again (the archive never shrinks).
- **Isolation.** The archiver's ClickHouse user can only `SELECT` from `logs`; its PostgreSQL role
  can only touch the `archive_*` tables (and read `projects`); its S3 identity can only use the
  archive bucket. The API's PostgreSQL role is read-only on the catalog and the API has no code
  path that writes or deletes objects. In production its S3 credentials should be read-only
  (`s3:GetObject`, `s3:ListBucket`). Locally its SeaweedFS identity also has `Write` on the archive
  bucket, because SeaweedFS 3.97 refuses object reads to identities without it (4.00 ignored the
  identity file in our test). Segments of deleted projects and segments past retention
  (365 days) are deleted by the archiver.
- **Dictionaries.** The archiver uploads a dictionary before it claims its version in the catalog,
  so a catalogued dictionary always exists; concurrent archivers cannot replace each other's bytes
  because dictionary keys contain a content hash.

## Why
- Logs are highly repetitive: a few hundred templates per service with changing numbers and IDs.
  Separating templates from variables (as in CLP) and storing columns by type leaves zstd mostly
  numbers and short strings, which compresses much better than whole lines. A per-service
  dictionary removes the cost of re-learning the same templates in every small segment.
- A separate process keeps archiving off the ingest and query paths and gives it its own,
  narrower credentials. ClickHouse's own S3 tiering would keep row-oriented parts with generic
  compression and could not use per-service dictionaries or templates.
- Object storage costs a fraction of ClickHouse disk and scales independently.

## Measured (local demo, 395k log rows, 32 windows)
- Archive: 56 bytes per row (161 MB of field data → 22.2 MB, 7.3×). ClickHouse with the new codecs:
  111.5 bytes per row (compressed parts of `logs`).
- Every demo row carries `log_id`, `trace_id` and `span_id`: 40 bytes of random data per row that
  no codec can shrink. That is most of the remaining size; templates, times and attributes
  take about 16 bytes per row.

## Not done (yet)
- Spans and events are not archived (the catalog has a `signal` column for them), so the map
  and request tracks are empty for ranges older than ClickHouse's TTL; only logs are shown there.
- Text search over the archive has no index (no token or n-gram filter per segment).
- The timeline keeps its 7-day range; long archive ranges are read through the log list.
