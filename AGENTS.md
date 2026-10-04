# AGENTS.md — working in the karbird repo

karbird is an observability, history-exploration and (later) controlled-management platform.

## Layout

| Path | What |
|---|---|
| `crates/karbird-domain` | IDs, scopes, redaction rules (shared, no I/O) |
| `crates/karbird-protocol` | Internal Kafka envelope (`proto/karbird/v1/ingest.proto`) |
| `crates/karbird-storage` | PostgreSQL (sqlx) and ClickHouse access, secret hashing |
| `crates/karbird-runtime` | tracing setup, shutdown, admin/metrics server |
| `crates/karbird-ingest` | OTLP HTTP/gRPC + events ingest → Kafka (ACK after broker ACK) |
| `crates/karbird-processor` | Kafka → ClickHouse, offsets in PostgreSQL |
| `crates/karbird-archive` | Log templates, `CHSEG` columnar segments, per-service zstd dictionaries, per-segment search index (pure, no I/O) |
| `crates/karbird-archiver` | Separate process: closed log windows ClickHouse → S3-compatible storage, catalog in PostgreSQL (ADR-0015) |
| `crates/karbird-api` | REST `/api/v1`, sessions, API tokens, actions API, WebSocket live, OpenAPI, `bootstrap` and `test-fixtures` subcommands |
| `crates/karbird-actions` | Action catalog, typed `Action`, validation, policy, state machine (pure, no I/O) |
| `crates/karbird-executor` | Separate process that runs approved plans (demo adapter) and verifies them; the only one allowed Docker |
| `crates/karbird-cli` | `karbird` CLI: doctor/discover/plan/apply/verify/context and the read-only `mcp` server |
| `crates/karbird-engine-core` | Timeline/topology engine (pure Rust, native + WASM) |
| `crates/karbird-web-engine` | wasm-bindgen wrapper around engine-core |
| `apps/web` | Next.js: public pages + private `/app` workspace |
| `packages/web-engine` | TS protocol, constants, ts-rs types, built WASM glue (`pkg/`) |
| `packages/api-client` | Generated OpenAPI types + typed fetch client |
| `packages/sdk-node` | `@karbird/sdk-node` (OpenTelemetry-based) |
| `examples/demo-system` | 7 services (gateway, auth, catalog, orders, inventory, payments, notifications) + Postgres + provider stand-in + load generator + fault injection |
| `infra/` | Docker Compose (incl. SeaweedFS as local S3), SQL migrations |
| `deploy/` | Production: Dockerfile + bake (GHCR images), Compose for the `storage` and `app` hosts, Caddyfile, host setup; driven by `.github/workflows/deploy.yml` (`docs/operations/deploy.md`, ADR-0017) |
| `scripts/` | Cross-platform Node scripts behind the `justfile` |
| `tests/integration`, `tests/e2e` | Node integration tests and Playwright tests against a running stack |

## Commands

```
just doctor          # tools, ports, bootstrap state
just bootstrap       # install deps, generate local secrets, start infra, migrate, apply retention, seed (needs .karbird/owner.env)
                     # retention days live in .karbird/settings.env; --shrink-retention confirms a decrease that deletes data (ADR-0016)
just dev             # ingest + processor + API + executor + archiver + web
just stop            # stop leftovers of just dev / just demo (keeps containers)
just reset           # DESTRUCTIVE: wipe data volumes, then bootstrap again (owner from .karbird/owner.env)
just demo            # demo system sending telemetry
just dev ngrok       # same as just dev, plus the web app at KARBIRD_PUBLIC_URL via ngrok (dev stand only)
just check           # fmt, clippy -D warnings, wasm32 check, TS typecheck, eslint
just test            # Rust unit/property tests + SDK tests
just test-integration / just test-e2e   # need `just dev` and `just demo` running
just test-broker-outage                 # disruptive (stops Kafka); run alone
just wasm / just codegen                # regenerate WASM artifact / contracts
```

## Rules

See `.cursor/rules/karbird-core.mdc` and `karbird-testing.mdc` (always apply) and `karbird-web.mdc`. In short:
- Keep the fixed data flow and ACK semantics. No LLM anywhere in the data path.
- Treat telemetry as untrusted. Redact at ingestion. Scope every query by the principal's tenant.
- No `unsafe`, no `unwrap` on external data, no TS `any`.
- Only `karbird-executor` may change the observed system or use Docker. New actions go into the
  `karbird-actions` catalog (typed, bounded, verified), never as free-form commands (ADR-0012).
- Every feature or fix ships tests at every applicable level in the existing suites: Rust `#[cfg(test)]`,
 vitest `*.test.ts`, `tests/integration/*.test.mjs`, Playwright `tests/e2e/specs/*.spec.ts`. A bug fix starts
 with a failing regression test. Never delete or weaken a test to get green.
- Never overwrite user changes, never `git reset`/`clean`, never commit `.karbird/`.
- Never `git commit` or `git push` (branches or tags): only the user commits and pushes.
- The only persistent account is the owner from `.karbird/owner.env`. Never seed other accounts in
  bootstrap; tests get theirs from `test-fixtures` (via `just test-integration` / `just test-e2e`), which
  deletes them after the run. Run a single spec with `node scripts/test-with-fixtures.mjs <command>`.
- Report only checks you actually ran.
