# ADR-0003: Consumer offsets in PostgreSQL, at-least-once

## Context
The processor needs a durable per-partition position, and rskafka has no group coordinator.
Exactly-once from Kafka into ClickHouse would need transactional sinks, which ClickHouse does not
provide.

## Decision
- Keep the offsets in the `consumer_offsets` table (consumer, topic, partition).
- For each batch: insert into ClickHouse, write undecodable records to `processing_errors`, and
  only then advance the offset.
- Accept at-least-once delivery and make writes idempotent (ADR-0004).

## Consequences
- A crash after the insert but before the offset update replays the batch. Duplicates collapse
  in ClickHouse.
- Offsets can be inspected and repaired with plain SQL.
