# Data flow

```
app + @controlhub/sdk-node ──OTLP/HTTP protobuf──┐
OTel Collector ──────────────OTLP/gRPC───────────┤
SDK events client ──────POST /api/v1/events/batch┘
                                                 ▼
                              controlhub-ingest (:4318 HTTP, :4317 gRPC, :9464 admin)
                                 1. bearer token → SHA-256 → PostgreSQL lookup (cached briefly)
                                 2. scope (tenant/project/environment) taken from the token only
                                 3. decode, validate limits, redact, attach scope
                                 4. produce IngestEnvelope to Kafka topic controlhub.telemetry.v1 (acks=-1)
                                 5. respond 200 only after the broker ACK; else 503 / gRPC UNAVAILABLE
                                                 ▼
                              controlhub-processor (:9465 admin)
                                 per partition: fetch → decode → insert into ClickHouse
                                 → record undecodable records in processing_errors
                                 → advance the offset in PostgreSQL (consumer_offsets)
                                                 ▼
                              ClickHouse: spans, logs, events (ReplacingMergeTree(ingested_at), TTL)
                                                 ▼
                              controlhub-api (:8080, /api/v1, cookie session, tenant-scoped queries)
                                                 ▼
                              Next.js (:3000) rewrites /api/v1/* → API
                                                 ▼
                              Web Worker: fetch JSON → WASM engine (apply, index, seek, frame)
                                                 ▼
                              UI thread: PixiJS map / table fallback, React panels, timeline
```

## Delivery guarantees

- **Ingest → Kafka.** ingest writes each request with `acks=-1` and returns success only after
  the broker acknowledges it. If the broker is unavailable, or does not ACK within 15 s, the
  client gets `503 Service Unavailable` with `Retry-After` (HTTP) or `UNAVAILABLE` (gRPC), and the
  item is counted in `failed_broker`. This was checked by stopping Kafka during a run: the demo
  services got 503, and once Kafka came back the counters recovered.
- **Kafka → ClickHouse** is at least once. The partition offset lives in PostgreSQL and moves
  forward only after the ClickHouse insert has succeeded. A crash can therefore replay a batch.
  Rows have deterministic keys (IDs are derived from the envelope, and span IDs come from OTLP),
  and the tables are `ReplacingMergeTree(ingested_at)`, so duplicates collapse on merge. Queries
  that must be exact use `FINAL`. See ADR-0004.
- **Undecodable records** do not block the partition. They are stored in `processing_errors`
  (payload capped at 1 MiB), counted, and shown through the coverage endpoint.
- **No silent drops.** Every item that is rejected, truncated or fails increments a counter,
  exposed at `:9464/metrics` for ingest and `:9465/metrics` for the processor. The SDK events
  client counts queued, sent, rejected, dropped, retried and failed items. There is one known gap:
  the OpenTelemetry batch processors inside the Node SDK drop spans or logs when their own queue
  is full, and these drops are not surfaced yet (see STATUS.md).

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
- Modes:
  - LIVE: poll every 2 s, with a 10 s overlap to catch late data.
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
- **Declared edges** (planned) will come from configuration. They are flagged separately and are
  never mixed with observed edges.
