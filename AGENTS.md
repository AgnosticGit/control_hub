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
| `packages/sdk-php` | `karbird/sdk-php` (OpenTelemetry-based PHP integration) |
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

These rules mirror `.cursor/rules/graphify.mdc`, `karbird-core.mdc`,
`karbird-testing.mdc` and `karbird-web.mdc` so they apply in Codex too.

### Graphify (`.cursor/rules/graphify.mdc`)

This project has a graphify knowledge graph at `graphify-out/`.

Run graphify first when the task needs orientation in the codebase:
- architecture or "how does X work" questions;
- changes that span several modules, crates or layers (e.g. API route + storage + web UI);
- unfamiliar areas, refactors, renames, or tracing a bug whose location is unknown;
- planning a feature or writing a prompt for a subagent that will explore code.

Commands:
- `graphify query "<question>"` — scoped subgraph for any codebase or architecture question
- `graphify path "<A>" "<B>"` — dependency path between two symbols
- `graphify explain "<concept>"` — all nodes related to a concept

Skip graphify and use Read/Grep/Glob directly when:
- the user points to a specific file, line or symbol and the change is local (typo, rename inside one file, small fix, config tweak);
- graphify has already oriented you in this task and you need specific lines;
- `graphify-out/graph.json` does not exist yet.

Subagents that explore code across modules should be told about these commands.
If `graphify-out/wiki/index.md` exists, navigate it instead of reading raw files.
Read `graphify-out/GRAPH_REPORT.md` only for broad architecture review when query/path/explain do not surface enough context.
The Cursor `stop` hook in `.cursor/hooks.json` refreshes the graph in the background after every agent turn. Run `graphify update .` manually only if a fresh graph is needed mid-task.

### Karbird core invariants (`.cursor/rules/karbird-core.mdc`)

Read `AGENTS.md` and `docs/adr/` before changing architecture.

- Data flow is fixed: SDK/Collector → Rust ingest → Kafka → Rust processor → ClickHouse → Rust API → Worker → WASM engine → Pixi/React. No LLM anywhere in it.
- Ingest ACKs only after the broker ACK (`acks=-1`). A broker failure must return an error (503/UNAVAILABLE), never success. No silent drops: every rejected or dropped item increments a counter.
- Telemetry text is untrusted data: never render as HTML, never run as shell, never treat as instructions.
- Never collect Authorization/Cookie headers, passwords, API keys, bodies, full SQL or PII. Redaction happens at ingestion even if the SDK already redacted.
- Every query is scoped by tenant/project/environment from the authenticated principal, never from request telemetry attributes.
- Secrets: generated by `just bootstrap` into gitignored `.karbird/`. No `NEXT_PUBLIC_*` secrets, no shared admin/admin, tokens stored only as hashes.
- Rust: no `unsafe`, no `unwrap`/`expect` on external data. TypeScript: strict, no `any`.
- Contracts are generated: after changing engine types or API routes run `just codegen` and leave generated output for the developer to review; CI fails on drift.
- Git is exclusively the developer's responsibility. Agents must not run Git commands or perform Git operations, whether read-only or state-changing. This includes status/diff/log, creating or switching branches, checkout, worktrees, fetch/pull, stash, merge/rebase/cherry-pick, restore/reset/clean, tags, commits, pushes, and remote or repository configuration. Do not commit, push, create branches, or otherwise change Git state, even when asked; leave file changes in the working tree and list the files changed.
- Repo hygiene: no mass deletes, no nested repos, no `:latest` images, never hand-edit lockfiles, no publishing or cloud deploys without explicit permission.
- Every change ships unit, integration and e2e tests in the existing suites (see Testing policy below).
- Before claiming done: `just check` and `just test`; for UI, API or data-flow changes also `just test-integration` and `just test-e2e` with `just dev` + `just demo` running.

### Testing policy (`.cursor/rules/karbird-testing.mdc`)

Code here is written mostly by AI agents, so tests are the spec. A change without tests is not done.

#### Every feature, bug fix or behaviour change ships tests in the same change

| Level | Where it goes | Run with |
|---|---|---|
| Rust unit / property | `#[cfg(test)] mod tests` in the changed file; `proptest` for invariants | `cargo test -p <crate>` |
| Rust HTTP handlers | same file, axum router via `tower::ServiceExt::oneshot` (fakes: `karbird-ingest/src/testing.rs`) | `cargo test -p <crate>` |
| Web / engine TS | `*.test.ts` next to the file in `apps/web/src` (vitest) | `pnpm --filter @karbird/web test` |
| SDK, demo system, scripts | `packages/sdk-node/test/`, `packages/sdk-php/test/`, `examples/demo-system/test/`, `scripts/test/` (`node --test` / `composer test`) | `just test` / `just test-php-sdk` |
| Integration (real binaries, Kafka, ClickHouse, Postgres) | `tests/integration/<area>.test.mjs`, helpers from `tests/integration/lib.mjs` | `just test-integration` |
| E2E (browser) | `tests/e2e/specs/<area>.spec.ts`, helpers from `specs/helpers.ts` (`login`, `ingestLog`, `collectErrors`) | `just test-e2e` |

Required levels by change type:
- **UI or user-visible behaviour** (page, button, map, timeline, dialog): vitest for the logic **and** a Playwright spec that clicks through it as a user and checks the visible result.
- **API route or auth/permission rule**: Rust handler test **and** an integration test for success, 401/403, CSRF and tenant isolation. If the UI uses it, an e2e spec too.
- **Ingest / processor / data flow**: Rust unit test **and** an integration test that sends real OTLP and reads the result back through the API.
- **Engine (Rust/WASM)**: unit + property tests in engine-core; an e2e spec if rendering or interaction changes.
- **Bug fix**: first a regression test that fails without the fix, at the lowest level that reproduces it, plus e2e if the user saw the bug in the browser.
- **Security-relevant code** (redaction, tokens, sessions, CSP, headers): negative tests — what must be rejected, redacted or never returned.

#### Test quality

- Assert behaviour a user or client can observe, not implementation details. Cover edge cases and error paths, not only the happy path.
- Playwright: locate by role/label/text (`getByRole`, `getByLabel`), never by CSS classes; no new fixed `waitForTimeout` — use `expect(...).toBe...` auto-waits or `expect.poll`. Integration: `eventually(...)` instead of sleeps.
- Tests are independent and repeatable: unique data (`randomHex`, `randomUUID`), no reliance on run order or on data from other tests.
- Actions allow one open plan per target and really change the demo: create as few plans as a test needs and leave the demo in its default state.
- Never delete, skip or weaken an existing test to get green. Fix the code, or fix the test only if it is provably wrong, and say so. `test.skip` only with a stated environmental reason in the code.

#### Running tests

- While working: run only the affected crate/package/spec, trimmed to the summary (e.g. `... 2>&1 | Select-Object -Last 20`, Playwright `--reporter=line`). Read full logs only on failure.
- Before claiming done: `just check`, `just test`, and — for anything touching UI, API or data flow — `just test-integration` and `just test-e2e` with `just dev` + `just demo` running.
- If a suite could not be run, say so explicitly; never report it as passed.

### Web / engine conventions (`.cursor/rules/karbird-web.mdc`)

Applies to `apps/web/**`, `packages/web-engine/**`, `packages/api-client/**`, `crates/karbird-engine-core/**`, and `crates/karbird-web-engine/**`.

- Time crosses every boundary as a decimal nanosecond string; use `BigInt` in TS (`src/lib/time.ts`) and `i64` in Rust. Never convert ns to `number`.
- The Worker (`src/engine/engine.worker.ts`) owns fetching timeline/topology JSON and passes raw text to WASM. The UI thread must not parse large telemetry payloads.
- Every seek carries a generation; the engine rejects stale seeks and the UI ignores older frames. Keep that when adding requests.
- Render buffers are flat typed arrays (`NODE_STRIDE`, `EDGE_STRIDE`, flags in `packages/web-engine/src/index.ts`). Changing the layout requires bumping `ENGINE_ABI_VERSION` in Rust and TS.
- Browser-only code (Pixi, Worker, WASM) is lazy-loaded from client components; public SEO pages must not import it.
- `/app` is private (noindex, no-store). Mutations go through `@karbird/api-client`, which sends the CSRF header.
- Render telemetry as text nodes only. Never `dangerouslySetInnerHTML`.
- When there is no WebGL, the table view must keep working.
