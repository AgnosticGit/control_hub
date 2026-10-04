# ADR-0014: OTLP metrics are not ingested (yet); service metrics come from spans

**Status: superseded by [ADR-0018](0018-infrastructure-metrics.md)** (machine and container
metrics are ingested; RED metrics still come from spans).

## Decision
- `POST /v1/metrics` (HTTP) and the gRPC metrics service stay explicitly **unimplemented**: HTTP
  answers 501, gRPC `UNIMPLEMENTED`. Nothing is accepted and silently dropped.
- RED metrics (request rate, errors, latency percentiles) shown on the map, in the inspector and
  used by action verification are computed from spans in ClickHouse (`service_summaries`).

## Why
- The product's core questions (what happened, in what order, what depends on what) are answered
  by traces, logs and events. Span-derived RED metrics cover the map and verification.
- Real metrics ingestion is a separate design: temporality (delta vs cumulative), histogram types,
  cardinality limits per tenant, a storage schema and downsampling, retention. Doing it halfway
  would mean unbounded cardinality from untrusted senders.

## When to revisit
When a use case needs data that spans cannot give (resource gauges such as CPU, memory and queue
depth, or runtime metrics). The prerequisites are per-tenant cardinality limits and counters for
rejected series, consistent with the no-silent-drop rule.
