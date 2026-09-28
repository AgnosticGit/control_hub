# Architecture decision records

| # | Decision | Status |
|---|---|---|
| [0001](0001-monorepo-layout.md) | Single Cargo + pnpm monorepo, scripts behind `just` | Accepted |
| [0002](0002-kafka-client-rskafka.md) | Kafka client: `rskafka` (pure Rust) instead of `rdkafka` | Accepted |
| [0003](0003-offsets-in-postgres.md) | Consumer offsets in PostgreSQL, at-least-once delivery | Accepted |
| [0004](0004-clickhouse-dedup.md) | ClickHouse dedup via `ReplacingMergeTree(ingested_at)` | Accepted |
| [0005](0005-wasm-engine-artifact.md) | WASM engine in a Worker; content-hashed artifact; ABI version | Accepted |
| [0006](0006-local-ports.md) | Non-default local infrastructure ports | Accepted |
| [0007](0007-browser-auth.md) | Cookie sessions + CSRF double-submit + Origin allowlist | Accepted |
| [0008](0008-typescript-version.md) | TypeScript 5.9 (not the native TS 7 preview) | Accepted |
| [0009](0009-worker-fetches-raw-json.md) | Worker fetches JSON and hands raw text to WASM | Accepted |
| [0010](0010-renderer-in-web-app.md) | Pixi renderer lives in `apps/web`, not a package | Accepted |
