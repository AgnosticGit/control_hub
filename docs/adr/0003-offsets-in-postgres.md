# ADR-0003: Consumer offsets in PostgreSQL, at-least-once

## Context
The processor needs a durable per-partition position and Kafka group coordination for horizontal
scaling. ClickHouse does not provide a transactional Kafka sink, so PostgreSQL remains the
authoritative processor watermark even though Kafka tracks group offsets too.
Exactly-once from Kafka into ClickHouse would need transactional sinks, which ClickHouse does not
provide.

## Decision
- Keep the offsets in the `consumer_offsets` table (consumer, topic, partition).
- For each record: insert into ClickHouse or durably write undecodable records to
  `processing_errors`, advance PostgreSQL, then commit the Kafka group offset.
- Accept at-least-once delivery and make writes idempotent (ADR-0004).

## Consequences
- A crash after the insert but before the offset update replays the batch. Duplicates collapse
  in ClickHouse.
- Offsets can be inspected and repaired with plain SQL.
- On assignment, the processor seeks to the PostgreSQL watermark. A Kafka commit failure can
  replay data but cannot move the durable watermark ahead of stored telemetry.
