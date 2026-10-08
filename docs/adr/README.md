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
| [0011](0011-api-tokens-and-live-channel.md) | Read-only, environment-bound API tokens; WebSocket live channel in the Worker | Accepted |
| [0012](0012-actions-and-executor.md) | Allowlisted actions, policy, approval by permission, separate executor, verification | Accepted |
| [0013](0013-read-only-mcp.md) | Read-only MCP server over the REST API with an API token | Accepted |
| [0014](0014-otlp-metrics-deferred.md) | OTLP metrics not ingested yet (explicit 501); RED metrics from spans | Superseded by 0018 |
| [0015](0015-log-archive.md) | Log archive: templates, columnar segments, per-service dictionaries, separate archiver | Accepted |
| [0016](0016-retention-config.md) | ClickHouse retention from one setting per signal, applied by bootstrap, checked at startup | Accepted |
| [0017](0017-two-host-deploy.md) | Prebuilt GHCR images, two-host Compose deploy from GitHub Actions, `migrate` instead of `bootstrap` | Accepted |
| [0018](0018-infrastructure-metrics.md) | Machine and container metrics (gauges/sums) from the Karbird agent; Infrastructure layer and noisy-neighbour rule | Accepted |
| [0019](0019-edge-traffic.md) | Link statistics from the engine; data flow from declared HTTP body sizes (unknown is not zero) | Accepted |
| [0020](0020-demo-platform-actions.md) | Container restart, scaling, feature flags and rollback as typed demo actions through a demo orchestrator; per-check verification | Accepted |
| [0021](0021-organisations-and-external-auth.md) | Organisations, explicit project assignments, Resend invitations and Google OIDC | Accepted |
| [0022](0022-persisted-server-topology.md) | Persisted backend graphs, shared geometry, fenced refresh and atomic client updates | Accepted |
