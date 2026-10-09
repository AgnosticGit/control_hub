# Kafka live fan-out design

## Goal

Remove per-WebSocket ClickHouse polling from the live telemetry path. Read each Kafka record once for live delivery, publish it to authorized WebSocket subscribers, and retain ClickHouse as the durable source for history and recovery.

## Current behavior

`karbird-api` handles each live WebSocket independently. Every second it issues three ClickHouse queries (spans, logs, events) for that socket. The queries intentionally stop three seconds behind wall-clock time to allow the processor's ClickHouse writes to become visible. Consequently ClickHouse query volume grows with open sockets and live delivery has a built-in delay.

The processor joins a Kafka consumer group, normalizes envelopes, writes ClickHouse, and advances its PostgreSQL offset only after successful writes. Kafka records are keyed by environment, preserving per-environment partition placement/order. The migration uses `rdkafka`/librdkafka for ingest production and all consumers.

## Approved direction

Migrate Kafka production and consumption to `rdkafka`. Add an independent live consumer inside `karbird-api`, with a consumer group identity distinct from the processor. It decodes envelopes, normalizes the spans/logs/events needed by the existing timeline contract, and broadcasts scoped batches through a bounded in-process hub to WebSockets. Each socket subscribes only to its authenticated tenant/project/environment. Browser clients continue to connect to the API over WebSocket and never connect to Kafka.

The live consumer is an independent Kafka consumer group, so it sees the same records while the processor continues storing them in ClickHouse. Every API replica must receive every partition to serve its local sockets. Therefore each API replica uses a distinct live `group.id` (one group per API instance); replicas must not share one group, which would divide partitions and leave some local sockets without updates. This duplicates Kafka reads per API replica but avoids work per browser and adds no new pub/sub service. A shared fan-out backbone is a future optimization if replica count makes duplicate reads material.

## Data and reliability contract

- Kafka is the source for new live items; ClickHouse remains the durable source for snapshots, history, and gap recovery.
- A live record is decoded and converted using the same canonical normalization rules as the processor. Avoid maintaining two divergent OTLP-to-row converters: extract/share the pure conversion code or create a shared conversion crate used by both API and processor.
- A Kafka live offset advances only after every item from the fetched record has entered the bounded fan-out hub. Auto commit is disabled. If the hub cannot accept the record, the consumer applies backpressure and does not commit its offset. On process restart, records may be replayed; stable item IDs and client deduplication make this at-least-once behavior safe.
- The processor retains PostgreSQL offsets as the authoritative completion watermark used by the API. It disables automatic Kafka commits, writes ClickHouse and PostgreSQL first, then commits the corresponding next Kafka offset. If the Kafka commit fails, replay is safe and the processor retries; if PostgreSQL persistence fails, it does not commit Kafka. Initial upgrade continues from existing PostgreSQL offsets and seeds broker group offsets as processing proceeds; it must not replay the entire retained topic unnecessarily.
- A slow socket must not block Kafka consumption or other sockets. Each socket has a bounded queue. If its queue overflows, send a `gap` control message (or close with a resumable code) and let the client recover through the existing HTTP/ClickHouse path.
- WebSocket resume cursors remain decimal nanosecond strings in the existing protocol. They are not Kafka offsets. Reconnect uses the current cursor; ClickHouse recovery repairs any gap and the live stream continues afterward. Since ingestion timestamps are not a total ordering key, batch ordering and equal-timestamp handling must be deterministic and dedupe must use item IDs.
- Authorization checks, origin validation, session rechecks, live-window limits, message limits, and existing gap/heartbeat behavior remain enforced.
- Unsupported/malformed envelopes are counted and logged safely; raw telemetry is never treated as instructions or rendered as HTML.

## Conversion boundary

Today `crates/karbird-processor/src/convert.rs` contains the canonical envelope decoder and conversion into ClickHouse row types, while `crates/karbird-api/src/routes/telemetry.rs` maps stored rows to `TimelineItem`. Live conversion must reuse the canonical processor mapping and timeline mapping. The implementation plan will extract the pure converter into a crate that both API and processor depend on, or use another design only if repository constraints show extraction is not viable. It must not introduce a second independently maintained interpretation of OTLP fields.

## Operational behavior

- The live consumer starts with the API and retries broker failures with bounded backoff; API startup must report a fatal configuration/start failure rather than silently claiming live health without a reader. The live consumer group ID is configured per API instance with a documented single-instance default.
- Aggregate Kafka lag, decode and offset-commit enqueue errors, hub backpressure, subscriber overflow, and scope eviction are exposed as low-cardinality Prometheus metrics at the API's private `/metrics` listener. The public Caddy site does not proxy this route.
- Shutdown/restart may replay records whose offsets were not committed. Only records admitted to the bounded fan-out hub are committed; the hub is in-memory and cannot promise delivery across a process crash. WebSocket clients repair gaps through ClickHouse-backed HTTP recovery.
- The API stays read-only with respect to ClickHouse on the steady-state live path. HTTP snapshot/history and recovery routes continue querying ClickHouse on demand.

## Non-goals

- Replacing Kafka, ClickHouse, the REST API, or the browser WebSocket protocol.
- A separate live service, Redis/NATS, or a new broker. API replicas each read a full copy under unique group IDs; a shared fan-out backbone can replace this if duplicate reads become material.
- Streaming unbounded raw metric points or historical query results through the live channel. Live infrastructure updates are bounded, aggregated snapshots.
- Exactly-once delivery. The contract is at-least-once with stable-ID deduplication and explicit gap recovery.

## Acceptance criteria

1. WebSocket sessions do not run periodic ClickHouse queries.
2. One API live reader consumes each configured Kafka partition independently of processor offsets.
3. Valid span, log, and event records become the same timeline item shapes as the ClickHouse-backed path.
4. Scope isolation prevents any tenant/project/environment from receiving another scope's records.
5. Bounded queues prevent slow clients from exhausting memory or stalling other clients; overflow is observable and recoverable.
6. Restart/replay, duplicate delivery, malformed records, broker outage, session revocation, and gap recovery are covered by tests.
7. Existing initial snapshot/history and HTTP recovery still work.
8. `just check`, `just test`, integration, and e2e suites pass where their required services are available.

## Workspace multiplexing extension (2026-10-09)

The workspace uses one authenticated, environment-scoped WebSocket for live spans, logs, events,
and optional infrastructure snapshots. Existing HTTP APIs remain the source for initial page data,
historical ranges, filtered/paginated searches, and trace details. The browser does not poll live
signal endpoints while the socket is healthy. A gap or reconnect uses the existing bounded HTTP
timeline recovery and refreshes the currently visible signal query.

- Live timeline items carry the complete safe DTO for spans, logs, and events in addition to the
  compact WASM timeline item. A bounded client ring and query-cache merge make the same live items
  visible in the side panels without parsing telemetry on the UI thread.
- Infrastructure snapshots are sent only when the connection opts into the infrastructure layer,
  at a 10-second cadence. The API shares/coalesces the ClickHouse-backed snapshot by exact scope,
  so many sockets for one environment cause one snapshot load per interval. No per-socket polling
  of ClickHouse is allowed.
- Infrastructure snapshot messages update only infrastructure state; topology remains on its
  existing revision-aware route. Initial selected-time/history snapshots continue using HTTP.
- Live message kinds remain explicitly typed (`batch`, `infrastructure`, and existing control
  messages); unknown kinds are ignored safely. Scope authorization, queue bounds, resume cursors,
  and session rechecks apply equally to every message kind.
- Historical search and pagination are intentionally HTTP queries. Live records are merged only
  into the first page when they satisfy the active service/severity/text/time filters; dedupe is by
  stable record ID. Trace summaries are incrementally merged from full span DTOs; trace details
  remain an on-demand HTTP request.

## Risks and decisions to validate during implementation

- The producer's queue-per-partition routing must be replaced with compatible key-based partitioning in librdkafka; verify all records for an environment remain ordered and consistently routed.
- Processor group assignment must resume from existing PostgreSQL offsets on upgrade, without replaying or skipping retained telemetry.
- Extracting the processor converter may affect crate dependencies and test placement. Keep the converter pure and avoid pulling ClickHouse I/O into API.
- Each API instance needs a full-copy live group because its in-process hub is local. Never use one shared live group across API replicas without cross-instance fan-out.
- The existing WebSocket cursor is ingestion-time based, while Kafka ordering is partition/offset based. Keep the two cursors distinct and use ClickHouse for temporal gap repair rather than pretending a timestamp is a Kafka offset.
