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
  container metrics come from the **Karbird Collector**: a pinned OpenTelemetry Collector (contrib) with
  `hostmetrics` and `docker_stats`, filtered to the metric names the API reads
  (`deploy/collector/`). One process per machine, not per service, so no service pays for it.
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
  with metrics; otherwise the machine is taken from `host.name` unverified (shown as "no collector
  metrics"), else unknown.
- **Noisy-neighbour rule** (deterministic, no LLM): an instance is flagged when its p95 over the
  last 5 minutes is at least 1.5× the p95 of the 30 minutes before **and** at least 20 ms higher,
  with at least 20 requests in each window, **and** on the same machine (with collector metrics) another
  container — not its own, not the same service — uses at least 80 % of the machine's CPU, or the
  machine is at least 90 % busy and that top consumer uses at least 50 %. The hint states which
  thresholds fired.
- **Timeline.** `GET …/environments/{env}/infrastructure/timeline?from=&to=&buckets=` (range up
  to 7 days, 1–1000 buckets, default 200) returns per machine the CPU busy share (increase of
  `system.cpu.time` busy vs idle; fallback `system.cpu.utilization`) and memory used share per
  bucket, plus container events derived from the same metrics — the collector needs no Docker events.
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
- **Databases and queues on our machines** (automatic, no manual binding, no LLM). The collector
  also reports each container's image (`container.image.name`) and its Docker Compose service
  (label `com.docker.compose.service` → `docker.compose.service`); no other labels and no
  environment. They are configuration names, not user data, and stored in `metrics`
  (`container_image`, `container_service`, ClickHouse migration `0004`). For every database or
  queue that client spans of the last 5 minutes call, the API reads the address the caller
  connected to (`server.address`, else `net.peer.name`, plus the port; never the statement) and
  where the caller runs (`host.name`, `host.id`, `container.id`, matched like instances). The
  address is lowercased, its trailing dot and IPv6 brackets dropped; addresses over 253
  characters are ignored. Per address, first match wins:
  1. `localhost`, `*.localhost`, `127.0.0.0/8`, `::1`, `host.docker.internal` → the caller's
     machine (only if the collector reports it), and on it the one container whose image runs that
     system (`postgresql`: postgres, postgis, timescale, pgvector; `mysql`: mysql, mariadb,
     percona; `redis`: redis, valkey, keydb, dragonfly; `mongodb`: mongo; `kafka`: kafka,
     redpanda; `elasticsearch`: elasticsearch, opensearch; `cassandra`: cassandra, scylla;
     otherwise the system's own name in the image's repository name);
  2. the Compose service of a container, then a container name, then a container hostname or
     short ID — on the caller's machine first, else unique across all machines;
  3. a machine's name or ID (with collector metrics) → that machine, container by image as in 1.

  Over all addresses of a resource: none matched but some exist → **external** (a managed or
  foreign database: Yandex Managed, Supabase, RDS, another network); matches on one machine →
  **internal** on it, with the container all matches agree on (two different containers or
  several of the same image → the machine only, never a guess); matches on several machines or
  no address at all → **unknown**. `GET …/infrastructure` returns this as `resources`
  (location, machine, container, rule, up to 5 addresses, a reason) and links the container
  (`resource_id`). The map puts internal resources into their machine's frame and draws external
  ones as external while the Infrastructure layer is on; the inspector shows "Runs on" for them.
  Matching by IP of the machine is not done: the collector does not report machine addresses.
- **Map.** The engine lays out the Infrastructure layer itself (machine frames around the service
  cards, a chip per instance of multi-instance services, `×N` badges, links from the slowed
  service to the noisy neighbour), so picking, culling and edges keep working. The UI shows the
  machine list, the hint, the machine card and the service → instance → container → machine chain.

## Consequences
- Machines without the collector still appear (from span `host.name`) but without numbers.
- To have its own databases shown as internal, a client runs the Collector (`deploy/collector/`) on
  every machine that hosts them and lets services connect by Compose service name, container
  name or `localhost`. A database reached by a public IP or DNS name of the same machine stays
  external until IP matching exists.
- The demo collector adds a synthetic `postgres:16-alpine` container on `vps-app`; orders and
  inventory reach the demo Postgres over `127.0.0.1`, so the loopback rule places it there.
- `just demo` runs a demo collector that reports the demo services as containers on two virtual
  machines from their real process CPU time and memory; `fault.mjs inventory --cpu-hog 0.95`
  reproduces a noisy neighbour.
- Application metrics (custom counters, histograms) are still out of scope; adding them needs
  histogram storage and per-tenant limits beyond the series cap above.
