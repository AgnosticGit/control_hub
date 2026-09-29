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
| `crates/controlhub-api` | REST `/api/v1`, sessions, API tokens, actions API, WebSocket live, OpenAPI, `bootstrap` subcommand |
| `crates/controlhub-actions` | Action catalog, typed `Action`, validation, policy, state machine (pure, no I/O) |
| `crates/controlhub-executor` | Separate process that runs approved plans (demo adapter) and verifies them; the only one allowed Docker |
| `crates/controlhub-cli` | `controlhub` CLI: doctor/discover/plan/apply/verify/context and the read-only `mcp` server |
| `crates/controlhub-engine-core` | Timeline/topology engine (pure Rust, native + WASM) |
| `crates/controlhub-web-engine` | wasm-bindgen wrapper around engine-core |
| `apps/web` | Next.js: public pages + private `/app` workspace |
| `packages/web-engine` | TS protocol, constants, ts-rs types, built WASM glue (`pkg/`) |
| `packages/api-client` | Generated OpenAPI types + typed fetch client |
| `packages/sdk-node` | `@controlhub/sdk-node` (OpenTelemetry-based) |
| `examples/demo-system` | 7 services (gateway, auth, catalog, orders, inventory, payments, notifications) + Postgres + provider stand-in + load generator + fault injection |
| `infra/` | Docker Compose, SQL migrations |
| `scripts/` | Cross-platform Node scripts behind the `justfile` |
| `tests/integration`, `tests/e2e` | Node integration tests and Playwright tests against a running stack |

## Commands

```
just doctor          # tools, ports, bootstrap state
just bootstrap       # install deps, generate local secrets, start infra, migrate, seed
just dev             # ingest + processor + API + executor + web
just stop            # stop leftovers of just dev / just demo (keeps containers)
just demo            # demo system sending telemetry
just check           # fmt, clippy -D warnings, wasm32 check, TS typecheck, eslint
just test            # Rust unit/property tests + SDK tests
just test-integration / just test-e2e   # need `just dev` and `just demo` running
just test-broker-outage                 # disruptive (stops Kafka); run alone
just wasm / just codegen                # regenerate WASM artifact / contracts
```

## Rules

See `.cursor/rules/controlhub-core.mdc` and `controlhub-testing.mdc` (always apply) and `controlhub-web.mdc`. In short:
- Keep the fixed data flow and ACK semantics. No LLM anywhere in the data path.
- Treat telemetry as untrusted. Redact at ingestion. Scope every query by the principal's tenant.
- No `unsafe`, no `unwrap` on external data, no TS `any`.
- Only `controlhub-executor` may change the observed system or use Docker. New actions go into the
  `controlhub-actions` catalog (typed, bounded, verified), never as free-form commands (ADR-0012).
- Every feature or fix ships tests at every applicable level in the existing suites: Rust `#[cfg(test)]`,
 vitest `*.test.ts`, `tests/integration/*.test.mjs`, Playwright `tests/e2e/specs/*.spec.ts`. A bug fix starts
 with a failing regression test. Never delete or weaken a test to get green.
- Never overwrite user changes, never `git reset`/`clean`, never commit `.controlhub/`.
- Report only checks you actually ran, and update `STATUS.md` with the real results.
