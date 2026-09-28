# ADR-0004: ClickHouse dedup via ReplacingMergeTree

## Context
With at-least-once delivery (ADR-0003) and client retries, the same span, log or event can arrive
more than once.

## Decision
- `spans`, `logs` and `events` use `ReplacingMergeTree(ingested_at)`. The sorting key, which is
  also the dedup key, is built only from deterministic fields:
  `(tenant, project, environment, service, start_time, trace_id, span_id)` for spans,
  `(…, service, time, log_id)` for logs and `(…, event_time, event_id)` for events.
  `log_id` is `<ingest batch_id>-<resource>-<scope>-<record>` and is therefore stable across
  Kafka redelivery. `event_id` is supplied by the client.
- Queries that need exact counts use `FINAL`. Retention is enforced with TTL (14 days for
  spans/logs, 30 days for events).

## Consequences
- Duplicates are removed at merge time, and `FINAL` hides them before that at some cost. This is
  fine at current volumes and will be revisited with benchmarks.
- Redelivery from Kafka to ClickHouse is fully deduplicated. Spans and events are also
  deduplicated when a client retries. A **log** export that a client retries after a lost ACK
  gets a new `batch_id`, so it is stored twice. The fix is a content hash or a client idempotency
  key (TASKS.md).
