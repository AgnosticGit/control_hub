# ADR-0018: Machine and container metrics over OTLP; Infrastructure layer on the map

Supersedes [ADR-0014](0014-otlp-metrics-deferred.md).

## Context
ADR-0014 deferred OTLP metrics until a use case needed data spans cannot give. The map now has
one: which machine a service runs on, how loaded that machine is, and whether a neighbour on the
same machine explains a slowdown ("noisy neighbour"). CPU, memory, disk and network of machines
and containers are resource gauges; spans cannot provide them.

## Decision
- **Who sends what.** The karbird SDK keeps exporting traces, logs and events only, and adds
  `host.name`, `container.id` and `process.pid` to the resource (`KARBIRD_HOST_NAME`,
  `KARBIRD_CONTAINER_ID`, else the OS hostname and `/proc/self/cgroup` / `mountinfo`). Machine and
  container metrics come from the **Karbird agent**: a pinned OpenTelemetry Collector (contrib) with
  `hostmetrics` and `docker_stats`, filtered to the metric names the API reads
  (`deploy/agent/`). One process per machine, not per service, so no service pays for it.
- **Ingest.** `POST /v1/metrics` and the gRPC `MetricsService` accept OTLP **gauges and sums**
  (cumulative or delta). Histograms, exponential histograms and summaries are rejected per point
  and reported in `partial_success`, as are invalid names and points over the per-request budget
  (20 000). Descriptions, metadata and exemplars are dropped; attributes go through the same
  redaction as spans. Tokens need the `metrics` signal (migration `0009` grants it to tokens that
  already had all other signals).
- **Cardinality.** Each ingest instance admits at most `KARBIRD_INGEST_MAX_SERIES_PER_ENV`
  (default 10 000) distinct series (name + resource + point attributes) per environment per hour.
  Points of further series are rejected and counted; with several ingest instances the effective
  limit is per instance.
- **Counters** (no silent drops): `karbird_ingest_items_total{signal="metrics",result=…}`,
  `karbird_ingest_metric_points_series_limited_total`, `karbird_processor_rows_total{table="metrics"}`,
  `karbird_processor_dropped_metric_points_total`.
- **Storage.** ClickHouse `metrics` (`ReplacingMergeTree(ingested_at)`, ordered by scope, metric
  name, series, time) with promoted `host_id`, `container_id`, `container_name`,
  `container_hostname` and `service_name`; retention `KARBIRD_METRICS_RETENTION_DAYS` (14,
  ADR-0016).
- **API.** `GET …/environments/{env}/infrastructure?at=` returns machines, their containers with
  CPU and memory shares, service instances and noisy-neighbour hints at `at`. Machine metrics are
  read over the last 2 minutes (rates from first/last cumulative values, counter resets give no
  rate). Instances come from request spans of the last 5 minutes and are matched to machines in
  this order: span `container.id` = metrics container ID (or its 12-hex short ID); span
  `host.name` = `container.hostname` or a short container ID; `host.id` / `host.name` = a machine
  with metrics; otherwise the machine is taken from `host.name` unverified (shown as "no agent
  metrics"), else unknown.
- **Noisy-neighbour rule** (deterministic, no LLM): an instance is flagged when its p95 over the
  last 5 minutes is at least 1.5× the p95 of the 30 minutes before **and** at least 20 ms higher,
  with at least 20 requests in each window, **and** on the same machine (with agent metrics) another
  container — not its own, not the same service — uses at least 80 % of the machine's CPU, or the
  machine is at least 90 % busy and that top consumer uses at least 50 %. The hint states which
  thresholds fired.
- **Timeline.** `GET …/environments/{env}/infrastructure/timeline?from=&to=&buckets=` (range up
  to 7 days, 1–1000 buckets, default 200) returns per machine the CPU busy share (increase of
  `system.cpu.time` busy vs idle; fallback `system.cpu.utilization`) and memory used share per
  bucket, plus container events derived from the same metrics — the agent needs no Docker events.
  Rules (deterministic, no LLM; 3 minutes before `from` are read so containers already running are
  not reported as started):
  - *started*: the container's first point is more than 90 s after its machine's first point;
  - *stopped*: the machine kept reporting more than 90 s after the container's last point;
  - *restarted*: `container.cpu.usage.total` went down (between buckets or within one);
  - *out_of_memory*: a stop or restart with memory at ≥ 95 % of the container's memory limit in
    that bucket or the one before;
  - *redeployed*: a stop and a start with the same name on the same machine and a different
    container ID, the stop within 5 minutes before (or 90 s after) the start — one event.

  Events are oldest first, at most 500 (`events_total` has the full count). The web timeline shows
  a "Machines" group with one row per machine (CPU bars on a fixed 0–100 % scale, red from 90 %,
  memory as a line; a value holds up to 90 s between exports) and a marker per event that moves the
  cursor there.
- **Map.** The engine lays out the Infrastructure layer itself (machine frames around the service
  cards, a chip per instance of multi-instance services, `×N` badges, links from the slowed
  service to the noisy neighbour), so picking, culling and edges keep working. The UI shows the
  machine list, the hint, the machine card and the service → instance → container → machine chain.

## Consequences
- Machines without the agent still appear (from span `host.name`) but without numbers.
- `just demo` runs a demo agent that reports the demo services as containers on two virtual
  machines from their real process CPU time and memory; `fault.mjs inventory --cpu-hog 0.95`
  reproduces a noisy neighbour.
- Application metrics (custom counters, histograms) are still out of scope; adding them needs
  histogram storage and per-tenant limits beyond the series cap above.
