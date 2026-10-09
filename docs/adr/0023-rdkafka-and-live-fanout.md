# ADR-0023: rdkafka and Kafka-backed live fan-out

## Context

The processor was reading every Kafka partition directly and each live WebSocket queried
ClickHouse every second. This made the processor awkward to scale and multiplied database reads
by the number of connected browsers. The Kafka records already contain the normalized source
envelope needed by both the processor and live timeline.

## Decision

- Use `rdkafka` with librdkafka and Tokio support for ingest production and Kafka consumption.
- The processor joins one consumer group (`karbird-processor`) and lets Kafka distribute
  partitions among replicas. PostgreSQL `consumer_offsets` remains authoritative: seek to its
  `next_offset` on assignment, persist ClickHouse/dead-letter outcomes, advance PostgreSQL, then
  asynchronously commit the Kafka group offset. Delivery remains at least once.
- Every API replica runs a separate consumer group and maintains a bounded, exact-scope local
  fan-out hub. `KARBIRD_LIVE_GROUP_ID` must be unique and stable per API replica; sharing one
  group between replicas would split partitions and leave each replica's local WebSocket clients
  without some updates.
- WebSockets subscribe to the hub before doing a bounded ClickHouse catch-up. That catch-up runs
  once per connection with overlap for delivery races and reconnect recovery. Steady-state items
  come from Kafka, not periodic ClickHouse queries. A full per-socket queue closes that stream
  and sends a `gap`, allowing the client to recover through the existing HTTP timeline endpoint.
- The API exposes aggregate live-reader lag, decode and offset-commit enqueue errors, and
  hub backpressure/overflow counters in Prometheus text format at `/metrics` on its private listener. The public Caddy site
  does not proxy this endpoint, and metrics contain no tenant/project/environment labels.
- Kafka and ClickHouse remain independent durable paths: Kafka is the live transport and replay
  buffer; ClickHouse remains the query/history store. The API's Kafka group starts at `latest`
  when new, then relies on the bounded ClickHouse catch-up for data that predates its group.

## Consequences

- Processor replicas can share partitions and rebalance automatically. ClickHouse inserts are
  still at least once and must remain idempotent.
- Open WebSockets no longer cause one ClickHouse query triplet per second. New and reconnecting
  sockets still perform one bounded catch-up; historical queries, topology refreshes, and
  infrastructure timeline reads continue using their existing HTTP/data paths.
- API replicas each read the complete Kafka topic. This duplicates Kafka reads across API
  replicas but avoids adding a separate shared pub/sub service; a shared backplane can be added
  later if the API replica count makes that cost material.
- The single API service in the production Compose file uses the stable group `karbird-live-api`.
  Before scaling it to multiple API replicas, provide a distinct stable `KARBIRD_LIVE_GROUP_ID`
  for each replica through the deployment orchestrator.
- Clean Rust builds need CMake and a C/C++ toolchain for librdkafka. The production build image
  installs these prerequisites; runtime images contain only the application binaries.
- A Kafka offset gap below the PostgreSQL watermark is recorded as a processing error and the
  processor advances to the earliest retained broker offset. Kafka retention must therefore be
  long enough for the expected outage/recovery window.
