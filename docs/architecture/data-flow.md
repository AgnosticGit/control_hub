# Data flow

```
app + @karbird/sdk-node or karbird/sdk-php ──OTLP/HTTP protobuf──┐
OTel Collector ──────────────OTLP/gRPC───────────┤
SDK events client ──────POST /api/v1/events/batch┘
                                                 ▼
                              karbird-ingest (:4318 HTTP, :4317 gRPC, :9464 admin)
                                 1. bearer token → SHA-256 → PostgreSQL lookup (cached briefly)
                                 2. scope (tenant/project/environment) taken from the token only
                                 3. decode, validate limits, redact, attach scope
                                 4. produce IngestEnvelope to Kafka topic karbird.telemetry.v1 (acks=-1)
                                 5. respond 200 only after the broker ACK; else 503 / gRPC UNAVAILABLE
                                                 ▼
                              karbird-processor (:9465 admin)
                                 per partition: fetch → decode → insert into ClickHouse
                                 → persist idempotent edge_span_facts for the live traffic chart
                                 → record undecodable records in processing_errors
                                 → advance the offset in PostgreSQL (consumer_offsets)
                                                 ▼
                              ClickHouse: spans, logs, events (ReplacingMergeTree(ingested_at), TTL)
                                                 ▼
                              karbird-api (:8080, /api/v1, cookie session or read-only API token,
                                              tenant-scoped queries, WebSocket …/live)
                                                 ▼
                              Next.js (:3000) rewrites /api/v1/* → API (HTTP and WebSocket)
                                                 ▼
                              Web Worker: fetch JSON / live WebSocket → WASM engine
                                          (apply, incremental append, index, seek, frame)
                                                 ▼
                              UI thread: PixiJS map / table fallback, React panels, timeline
```

Side paths (not part of the telemetry flow, and no LLM anywhere):

```
web (Actions page) ─propose/approve─▶ karbird-api ─▶ PostgreSQL action_plans + audit_log
                                                               ▲ claim (SKIP LOCKED, lease)
karbird-executor ───────────────────────────────────────────┘
   ├─ demo adapter: loopback /__demo/faults, docker restart (fixed args, label check)
   └─ verification: ClickHouse service_summaries (SELECT-only user)

karbird CLI `context` / `mcp` ──read-only API token──▶ karbird-api (GET only)
```

## Delivery guarantees

- **Ingest → Kafka.** ingest writes each request with `acks=-1` and returns success only after
  the broker acknowledges it. If the broker is unavailable, or does not ACK within 15 s, the
  client gets `503 Service Unavailable` with `Retry-After` (HTTP) or `UNAVAILABLE` (gRPC), and the
  item is counted in `failed_broker`. `just test-broker-outage` checks this automatically. It
  stops Kafka and expects 503 ("events NOT accepted, retry") after the 15 s ACK timeout, with a
  higher `failed_broker` counter. It then restarts Kafka and requires a new event to be accepted
  and queryable.
- **Kafka → ClickHouse** is at least once. The partition offset lives in PostgreSQL and moves
  forward only after the ClickHouse insert has succeeded. A crash can therefore replay a batch.
  Rows have deterministic keys (IDs are derived from the envelope, and span IDs come from OTLP),
  and the tables are `ReplacingMergeTree(ingested_at)`, so duplicates collapse on merge. Queries
  that must be exact use `FINAL`. See ADR-0004.
- The processor and live reader use the ingest envelope's `received_time_unix_nano` as the stable
  storage cursor. This differs from the WebSocket resume cursor, which orders live delivery.
  `edge_span_facts` is written before the processor commits its PostgreSQL offset, uses stable span
  identity and backs the edge traffic query; redelivery replaces a fact instead of incrementing it.
- **Undecodable records** do not block the partition. They are stored in `processing_errors`
  (payload capped at 1 MiB), counted, and shown through the coverage endpoint.
- **No silent drops.** Every item that is rejected, truncated or fails increments a counter,
  exposed at `:9464/metrics` for ingest and `:9465/metrics` for the processor. The SDK events
  client counts queued, sent, rejected, dropped, retried and failed items. There is one known gap:
  the OpenTelemetry batch processors inside the Node SDK drop spans or logs when their own queue
  is full, and these drops are not surfaced yet.

## Timeline and time machine

- The API's `/timeline` returns spans, logs and events for a range as one list with
  nanosecond-string timestamps. It also returns a watermark and a completeness indicator, which
  the UI shows as a badge.
- The Worker fetches the JSON itself and hands the raw text to WASM, so the UI thread never parses
  large payloads.
- The engine keeps sorted per-kind indexes and a checkpoint every N items. `seek(t)` restores the
  nearest checkpoint and replays forward.
- Every seek carries a generation number. The engine drops stale seeks, and the UI ignores frames
  from older generations. Seeks, viewport changes and filter changes that arrive in the same tick
  are merged into a single frame.
- Live telemetry frames are merged into the existing timeline index incrementally. A full rebuild happens
  only on structural changes: a new node or edge (which changes the state-vector layout), or a
  parent span that arrives after its child. The diagnostics popover counts both merges and
  rebuilds, and a property test checks that incremental merges equal a full build.
- The selected edge chart starts from the durable ClickHouse aggregate and applies newly delivered
  matching span records from the same WebSocket. Its storage cursor keeps catch-up and reconnects
  from counting records already included in the chart baseline.
- Modes:
  - LIVE: WebSocket `…/live` owned by the Worker (`hello`/`span`/`log`/`event`/`gap`/`heartbeat`),
    with a resume cursor on reconnect. The API pages durable ClickHouse catch-up results and sends
    each record in its own frame. A closed or delayed socket marks the view stale and reconnects
    with backoff; live telemetry never falls back to periodic HTTP polling.
  - PAUSED: stops following live data.
  - REPLAY: a requestAnimationFrame clock at 0.1–10x.
  - HISTORICAL: an explicit range of at most 7 days.
  - Compare A/B: the engine diffs node metrics between two moments.

## Topology

- **Observed edges** come from spans: a CLIENT or PRODUCER span in service A whose child SERVER or
  CONSUMER span is in service B.
- **Inferred edges** go to resources such as databases or peers named in client span attributes
  (`db.system`, `server.address`, …). The map draws them dashed and lists the reasons they were
  inferred.
- **Systems outside the project** are inferred too and drawn as light, dashed "external" cards
  with a globe icon; the inspector says they are not part of the project:
  - *External service* (`external_dependency`): an HTTP CLIENT span (`http.request.method`) to a
    host (`server.address`/`server.port`, else the URL authority; a non-default port is part of
    the name, path/query/credentials never are). The host is dropped when any call to it in the
    window was answered by an instrumented service, so internal calls stay internal even while
    the newest answers are still in an export batch. A down internal service can briefly look
    external.
  - *External clients* (`external_client`): one node per environment for requests that arrived
    without an instrumented caller (root SERVER spans: browser, frontend, curl, partner). Timeline
    items of such spans carry `source_entity_id`, so the engine animates the request from that
    node to the service and the answer back.
  - Their metrics come from the calling / receiving service's spans; external clients are not
    counted again in the summary totals. HTTP only for now (no gRPC peers).
- **Declared edges** (planned) will come from configuration. They are flagged separately and are
  never mixed with observed edges.
