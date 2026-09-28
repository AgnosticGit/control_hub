# AGENTS.md — working in the ControlHub repo

ControlHub is an observability, history-exploration and (later) controlled-management platform.
Current progress lives in `STATUS.md`, planned work in `TASKS.md`, and decisions in `docs/adr/`.

## Layout

| Path | What |
|---|---|
| `crates/controlhub-domain` | IDs, scopes, redaction rules (shared, no I/O) |
| `crates/controlhub-protocol` | Internal Kafka envelope (`proto/controlhub/v1/ingest.proto`) |
| `crates/controlhub-storage` | PostgreSQL (sqlx) and ClickHouse access, secret hashing |
| `crates/controlhub-runtime` | tracing setup, shutdown, admin/metrics server |
| `crates/controlhub-ingest` | OTLP HTTP/gRPC + events ingest → Kafka (ACK after broker ACK) |
| `crates/controlhub-processor` | Kafka → ClickHouse, offsets in PostgreSQL |
| `crates/controlhub-api` | REST `/api/v1`, sessions, OpenAPI, `bootstrap` subcommand |
| `crates/controlhub-engine-core` | Timeline/topology engine (pure Rust, native + WASM) |
| `crates/controlhub-web-engine` | wasm-bindgen wrapper around engine-core |
| `apps/web` | Next.js: public pages + private `/app` workspace |
| `packages/web-engine` | TS protocol, constants, ts-rs types, built WASM glue (`pkg/`) |
| `packages/api-client` | Generated OpenAPI types + typed fetch client |
| `packages/sdk-node` | `@controlhub/sdk-node` (OpenTelemetry-based) |
| `examples/demo-system` | gateway + orders + Postgres + load generator + fault injection |
| `infra/` | Docker Compose, SQL migrations |
| `scripts/` | Cross-platform Node scripts behind the `justfile` |
| `tests/integration`, `tests/e2e` | Node integration tests and Playwright tests against a running stack |

## Commands

```
just doctor          # tools, ports, bootstrap state
just bootstrap       # install deps, generate local secrets, start infra, migrate, seed
just dev             # ingest + processor + API + web
just demo            # demo system sending telemetry
just check           # fmt, clippy -D warnings, wasm32 check, TS typecheck, eslint
just test            # Rust unit/property tests + SDK tests
just test-integration / just test-e2e   # need `just dev` and `just demo` running
just wasm / just codegen                # regenerate WASM artifact / contracts
```

## Rules

See `.cursor/rules/controlhub-core.mdc` (always applies) and `controlhub-web.mdc`. In short:
- Keep the fixed data flow and ACK semantics. No LLM anywhere in the data path.
- Treat telemetry as untrusted. Redact at ingestion. Scope every query by the principal's tenant.
- No `unsafe`, no `unwrap` on external data, no TS `any`.
- Never overwrite user changes, never `git reset`/`clean`, never commit `.controlhub/`.
- Report only checks you actually ran, and update `STATUS.md` with the real results.
