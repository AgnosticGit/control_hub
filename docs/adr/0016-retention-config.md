# ADR-0016: ClickHouse retention configured once, applied by bootstrap, checked at startup

## Context
The ClickHouse TTLs of `logs`, `spans` and `events` were fixed in migration 0001 (14/14/30 days).
The archiver (ADR-0015) and the API also depend on the log TTL: the archiver plans which windows
are still complete in ClickHouse, and the API switches to the archive before ClickHouse deletes
rows. Each had its own copy of the number (`KARBIRD_HOT_RETENTION_DAYS`,
`KARBIRD_ARCHIVE_HOT_WINDOW_HOURS`), so a change in one place silently broke the others.

## Decision
- **One setting per signal:** `KARBIRD_LOGS_RETENTION_DAYS` (default 14),
  `KARBIRD_SPANS_RETENTION_DAYS` (14), `KARBIRD_EVENTS_RETENTION_DAYS` (30),
  `KARBIRD_METRICS_RETENTION_DAYS` (14, added with ADR-0018), 2 to 3650 days.
  Locally they live in `.karbird/settings.env`, created with defaults by `just bootstrap` and
  never overwritten. Every binary loads it before the generated `services.env`; real environment
  variables win over both.
- **`karbird-api bootstrap` applies them** after the ClickHouse migrations:
  `ALTER TABLE … MODIFY TTL toDateTime(<time column>) + INTERVAL N DAY`, only when the value
  differs from the table's TTL (read from `system.tables.engine_full`). Lengthening applies at
  once. Shortening applies at once only when no rows are older than the new value; otherwise it
  stops with the number of rows that would be deleted and needs
  `just bootstrap --shrink-retention`. A TTL in any other shape is reported, never overwritten.
- **Services check, never change.** At startup the API (all three tables) and the archiver
  (`logs`) compare the configuration with the real TTL and refuse to start on a mismatch, naming
  the variable, both values and `just bootstrap`. Their read-only ClickHouse users can read the
  TTL of the tables they may select from; only bootstrap's admin user alters tables. The API waits
  for ClickHouse to come up (bounded retries) but fails at once on a mismatch.
- **The API's hot window follows the log retention:** one day less by default.
  `KARBIRD_ARCHIVE_HOT_WINDOW_HOURS` is an optional override and must stay below the
  retention, otherwise the API does not start.
- Table and column names come from a fixed list (`karbird-storage/src/retention.rs`); the days
  are validated integers, so the `ALTER` text has no external input.

## Consequences
- Changing retention is: edit the value, run `just bootstrap`, restart. Forgetting the bootstrap
  shows up as a startup error, not as lost archive windows or empty query ranges.
- `ALTER … MODIFY TTL` rewrites parts in the background (ClickHouse mutation); on large tables the
  deletion after a decrease takes time, but the new TTL is in effect immediately.
- Bootstrap is now required after a retention change in every deployment, not only locally.
