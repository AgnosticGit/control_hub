# ControlHub — status

Last updated: 2026-09-29. **ControlHub is not complete and not production-ready.** This file
lists only what has been built and actually checked, with real results.

## Summary

| Stage | State |
|---|---|
| 0. Repository and decisions | Done: workspace, pinned toolchain, README/AGENTS/ADRs 0001–0014, `justfile`, CI workflow (written, never run on GitHub) |
| 1. First end-to-end flow | Done and verified locally, including an automated broker-outage test |
| 2. Useful viewer | Done for the demo scale: WebSocket live channel with reconnect/resume/polling fallback, incremental engine merge. Open: server-side live filters, renderer LOD for big graphs |
| 3. Timeline | Done: modes, compare, completeness, reload restores history (e2e). Open: late-data indicator, snapshots |
| 4. Integration via coding agent | Done for Node.js: CLI (`doctor/discover/plan/apply/verify/context`), fixture project test, token management, Cursor docs. SDK and CLI are not published |
| 5. Controlled actions | Done for the **demo adapter only**: plans, policy, four-eyes approval, separate executor, verification from telemetry, audit, web page, integration + e2e tests. No real (non-demo) adapters |
| 6. Public site and expansion | Public pages, read-only MCP server (`controlhub mcp`), strict CSP for `/app`. **Not done:** archiving, TLS, production deployment |

## What works (verified)

- **Local infrastructure.** Docker Compose: PostgreSQL 17.6, ClickHouse 25.8, Kafka 4.1.0 (KRaft)
  and a demo Postgres. Everything binds to 127.0.0.1, and no image uses `:latest`.
  `just bootstrap` generates every secret into gitignored `.controlhub/`, creates least-privilege
  users per service (ingest, processor, API, executor) in PostgreSQL and ClickHouse, applies
  migrations 0001–0003, and seeds:
  - two tenants: demo, plus isolation with the non-demo environment `isolation-prod`;
  - users: admin, viewer, operator, isolation.
- **Ingest.**
  - OTLP/HTTP protobuf and OTLP/gRPC for traces and logs, plus custom events.
  - Redaction at ingestion. Scope always comes from the token.
  - `acks=-1`; the 200 is sent only after the broker ACK.
  - OTLP metrics are explicitly 501 (ADR-0014).
- **Broker outage (automated).** `just test-broker-outage` stops Kafka, gets
  `503 {"error":"broker did not acknowledge; events NOT accepted, retry"}` after 15.0 s, sees
  `failed_broker` go up, restarts Kafka, and checks that a new event is accepted and queryable.
- **Processor.** Kafka → ClickHouse with offsets in PostgreSQL. `log_id` is now a content hash,
  so client retries of logs deduplicate (ADR-0004).
- **API.**
  - Sessions with CSRF and an Origin allowlist.
  - Tenant-scoped read endpoints.
  - WebSocket `…/live`: Origin check for sessions, credential re-check every 30 s, connection
    limit.
  - Token management (ingest and API tokens: create/list/revoke/rotate with a grace period,
    audited).
  - Read-only, environment-bound API tokens (ADR-0011).
  - Actions API: catalog/list/detail/propose/approve/reject/cancel (ADR-0012).
- **Executor** (`controlhub-executor`, started by `just dev`):
  - Claims approved plans (`FOR UPDATE SKIP LOCKED`, 60 s lease) and re-checks validation and
    policy.
  - Runs the demo adapter:
    - fault inject/clear via the loopback endpoint, with read-back;
    - restart of demo Postgres via `docker` with fixed args after a compose-label check.
  - Verifies from ClickHouse over a 15 s window plus an 8 s settle time.
  - Records the result with an audit entry. Expired plans are never run.
- **CLI** (`controlhub`): discover/plan/apply (allowlisted files, drift detection, `--yes`),
  verify (end-to-end event plus app telemetry), context (bounded, sanitized), doctor, and `mcp`.
- **MCP** (`controlhub mcp`, ADR-0013): stdio JSON-RPC with 8 read-only query tools. Arguments
  are validated, output is sanitized, and every result carries an untrusted-data notice. There
  are no mutation tools.
- **Web.**
  - Public pages.
  - Workspace: Pixi map, panels, timeline, live badge ("Live: WebSocket" / "Live: polling").
  - Tokens page (admin) and Actions page (propose / approve / reject / cancel; detail with policy
    checks, result, verification, audit trail).
  - CSP: strict nonce policy on `/app/*` (no `unsafe-eval`/`unsafe-inline` for scripts),
    baseline policy elsewhere.
- **Demo.** Gateway → orders → Postgres, load generator, fault injection (loopback,
  `CONTROLHUB_DEMO_FAULTS=1`).

### Bugs found and fixed by the new tests (this session)
- **Demo `orders` crashed on a database restart.** The `pg` pool emitted an unhandled `error`
  event, and the whole demo supervisor exited. Executor verification caught it ("orders received
  0 requests") and marked the plan `verification_failed`. Fixed with a pool error handler; the
  restart action now passes.
- **Duplicate OpenAPI `operationId` `list`** (tokens and actions) broke the generated TS client.
  The action routes now have explicit operation IDs, and their schemas are typed (`PolicyDto`,
  numeric params).
- **e2e specs ran on parallel workers against one shared demo system**, so fault injection in one
  spec made verification in another fail (again correctly detected: gateway error ratio 27.4%).
  Playwright now uses `workers: 1`.
- **Executor container-name check** accepted names starting with `-`, which docker could parse as
  a flag. Names must now start with an alphanumeric character (unit test).

## Checks run in this session (real results)

Machine: Windows 11 (10.0.26200), Rust 1.98.1, Node 24.16.0, pnpm 10, Docker Desktop.
Stack: `node scripts/dev.mjs --no-web` (ingest, processor, API, executor), `just demo`, and the
web production build via `pnpm --filter @controlhub/web start`.

| Command | Result |
|---|---|
| `just check` (fmt check, clippy `-D warnings` for the workspace, wasm32 check, TS typecheck, eslint) | pass |
| `just test` (`cargo test --workspace` + SDK tests) | pass: 58 Rust tests (actions incl. proptest, API, CLI incl. MCP protocol tests, executor adapter, engine, storage, …), SDK tests pass |
| `just codegen` | run after the actions routes; output committed to the working tree |
| `just bootstrap` | pass: migration 0003, executor roles, new seed users |
| `pnpm --filter @controlhub/web build` | pass (13 routes; `/app/*` dynamic for CSP nonces) |
| `node --test "tests/integration/*.test.mjs"` | pass: **31/31, 1 skipped** (the disruptive broker-outage test). Includes actions 5/5 (with DB restart), MCP 4/4, CLI 4/4, tokens 4/4, WebSocket, isolation |
| `just test-broker-outage` | pass: 1/1 (503 after 15.0 s, recovery) |
| `pnpm --filter @controlhub/e2e exec playwright test` | pass: **16/16** (actions 2, CSP 2, tokens 2, workspace incl. live/reload, fault scenario, isolation) |

## Not run, and why

- **CI** (`.github/workflows/ci.yml`): a GitHub remote exists (`origin`, one commit `upd1`), but I
  have not checked any workflow run (`gh` is not installed here). None of this session's changes
  are committed or pushed; that is the user's call.
- **A bootstrap from a clean checkout**, and **Linux/macOS**, have not been tried; only this
  Windows working tree was used.
- **Load or soak tests** beyond the demo load (about 5 RPS) have not been run; there are no
  throughput numbers.
- **Executor loss** (lease expiry → `failed`, never retried): the SQL and the loop exist, but no
  test kills the executor mid-plan.
- **`just bench`** was not re-run after the incremental engine change. The earlier numbers
  (155k items, first frame 148 ms, seek 0.015 ms) are from before.

## Known limitations

- Actions exist only for the demo system (3 actions). Verification is statistical over 15 s: it
  shows the service receives traffic with a bounded error ratio, not causality.
- Policy limits are fixed in code (30 plans/hour per environment, one open plan per target,
  30 min decision window, 10 min start window). There is no per-tenant configuration.
- The MCP server is stdio only (no remote transport). MCP and `context` see one environment per
  token.
- The live channel streams the whole environment. Filtering happens in the engine, not on the
  server.
- Public static pages use a baseline CSP that still allows inline scripts; only `/app/*` has the
  strict nonce policy.
- OTLP metrics are not ingested (ADR-0014). There is no TLS in dev. Logins are audited
  best-effort only. There is no user management UI or API.
- The OpenTelemetry batch processors in the Node SDK can drop data when full, without a counter.
- The processor does not coordinate consumer groups (one instance reads all partitions).
- The Worker still reads timeline metadata from the response tail (ADR-0009).
- `@controlhub/sdk-node` and the CLI are not published. Projects reference them from a checkout.

## Next concrete task

Commit and push (user decision), get CI green, then verify `just bootstrap` from a clean checkout
on Linux.
After that: an automated executor-loss test, and server-side filters on the live channel. See
`TASKS.md`.
