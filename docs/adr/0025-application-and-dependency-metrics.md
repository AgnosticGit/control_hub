# ADR-0025: Application histograms and read-only dependency metrics

## Context

ADR-0018 limited OTLP metrics to gauges and sums for host and container placement. It rejected
histograms, so HTTP latency from an instrumented application could not be compared with runtime
or database state in the inspector.

## Decision

- The Node SDK enables its OpenTelemetry metrics reader. HTTP instrumentation exports
  `http.server.request.duration` histograms; bounded process gauges/counters report RSS, heap
  use and CPU time. Host and container metrics remain the Collector's responsibility.
- Ingest accepts explicit-boundary OTLP histograms only when bounds are finite, strictly
  increasing, at most 64, and bucket counts add up to the reported count. It removes exemplars,
  redacts attributes and charges every stored bucket against the request budget. Exponential
  histograms and legacy summaries remain rejected and counted.
- The processor stores each histogram point as `metric.bucket`, `metric.count` and `metric.sum`
  rows in the existing scoped `metrics` table. Each bucket carries its upper bound in `le`;
  the kind retains delta or cumulative temporality. A broker replay replaces the same rows.
- A separate optional PostgreSQL Collector uses a read-only monitoring account and exports
  statistics through OTLP. Query text and query samples are not enabled. The local demo collector
  reads `pg_stat_database` directly so its PostgreSQL card reflects the real demo database.
- `GET .../metrics/overview?name=&at=` uses the authenticated project/environment scope and
  a five-minute window. Cumulative streams need at least two points to produce rates or a
  histogram percentile. The p95 is the upper bound of the first bucket reaching 95%, not an
  exact observed latency. Missing data stays missing.
- The inspector shows application HTTP and process metrics and, when an observed PostgreSQL
  dependency is linked, its connection and transaction metrics alongside the service.

## Limits

The current view summarizes a bounded set of metric names and series. It does not offer a
general-purpose metric query language. The first window after a process start may lack enough
cumulative points for p95 or rates. Per-service metrics require the Node SDK or another sender
using the same OTLP metric names; PostgreSQL collection needs read-only database access.
