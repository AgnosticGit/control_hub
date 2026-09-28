# ADR-0006: Local ports

## Decision
- Infrastructure is published only on 127.0.0.1, on non-default ports: PostgreSQL 15432,
  ClickHouse HTTP 18123, Kafka 19092, demo Postgres 15433.
- The ControlHub services use their conventional ports: OTLP 4318/4317, API 8080, web 3000.
- `just doctor` reports which of these ports are busy.

## Consequences
- The stack does not clash with a developer's own Postgres, ClickHouse or Kafka.
- Nothing is reachable from the LAN by default.
