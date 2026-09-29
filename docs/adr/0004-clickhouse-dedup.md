# ADR-0004: ClickHouse dedup via ReplacingMergeTree

## Context
With at-least-once delivery (ADR-0003) and client retries, the same span, log or event can arrive
more than once.

## Decision
- `spans`, `logs` and `events` use `ReplacingMergeTree(ingested_at)`. The sorting key, which is
  also the dedup key, is built only from deterministic fields:
  `(tenant, project, environment, service, start_time, trace_id, span_id)` for spans,
  `(…, service, time, log_id)` for logs and `(…, event_time, event_id)` for events.
  `log_id` is a content fingerprint: the first 128 bits of SHA-256 over length-prefixed service
  id, instance, time, observed time, severity, body, trace/span id and sorted attributes. It is
  therefore stable across Kafka redelivery **and** client retries. (It used to be
  `<batch_id>-<resource>-<scope>-<record>`, which changed on every retry.) `event_id` is supplied
  by the client.
- Queries that need exact counts use `FINAL`. Retention is enforced with TTL (14 days for
  spans/logs, 30 days for events).

## Consequences
- Duplicates are removed at merge time, and `FINAL` hides them before that at some cost. This is
  fine at current volumes and will be revisited with benchmarks.
- Redelivery from Kafka to ClickHouse is fully deduplicated, and so are client retries of spans,
  logs and events (unit test `retried_log_export_gets_the_same_log_id`).
- Trade-off: two genuinely identical log records (same instance, same nanosecond timestamps, same
  body and attributes) collapse into one row. This is acceptable because they are
  indistinguishable anyway.
