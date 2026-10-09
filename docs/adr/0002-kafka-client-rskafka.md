# ADR-0002: Kafka client — rskafka (superseded)

## Context
The spec requires a Kafka-compatible client, and ingest must ACK only after the broker ACK. The
main candidates were `rdkafka`, which binds librdkafka (C, needs cmake/a C toolchain and is
painful to build on Windows), and `rskafka`, which is pure Rust and async (Tokio).

## Historical decision
This choice was replaced by [ADR-0023](0023-rdkafka-and-live-fanout.md). The current client is
`rdkafka` (librdkafka) with Tokio support and native build prerequisites.

## Consequences
- Historical rationale and tradeoffs are retained here for context only.
