# ADR-0002: Kafka client — rskafka

## Context
The spec requires a Kafka-compatible client, and ingest must ACK only after the broker ACK. The
main candidates were `rdkafka`, which binds librdkafka (C, needs cmake/a C toolchain and is
painful to build on Windows), and `rskafka`, which is pure Rust and async (Tokio).

## Decision
Use `rskafka` 0.6:
- Produce with `acks=-1` (all in-sync replicas) and a 15 s timeout per request.
- Records are protobuf `IngestEnvelope`s, keyed by `tenant/project/environment` so that
  one environment stays within one partition, which keeps its data in order.
- The consumer is written by hand per partition, with offsets stored in PostgreSQL (ADR-0003),
  because rskafka has no consumer-group coordination.

## Consequences
- The build has no native dependencies, and Windows and Linux builds behave the same.
- There is no consumer-group rebalancing. The processor reads every partition itself, so scaling
  out means assigning partitions explicitly; this is future work.
- Only brokers compatible with the protocol subset rskafka supports will work (Kafka ≥ 2.x,
  Redpanda).
