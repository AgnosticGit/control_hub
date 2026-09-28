# ControlHub — status

Last updated: 2026-09-29. **ControlHub is not complete.** This file lists only what has been
built and actually checked, with real results.

## Summary

| Stage | State |
|---|---|
| 0. Repository and decisions | Done: workspace, pinned toolchain, README/AGENTS/ADRs, `justfile`, CI workflow (written but never run on GitHub) |
| 1. First end-to-end flow | Done and verified locally |
| 2. Useful viewer | Mostly done. WebSocket realtime and reconnect handling are **not** done; live mode polls every 2 s |
| 3. Timeline | Mostly done: seek/play/pause/step/speeds/range/compare, completeness badge, synchronized panels, property tests + e2e. Incremental timeline append is not done |
| 4. Integration via coding agent | Only the Node SDK is done. CLI, integration plan, fixture project, apply/verify: **not started** |
| 5. Controlled actions | **Not started** (only the `audit_log` table and endpoint exist) |
| 6. Public site and expansion | Public SEO pages, robots/sitemap and short docs pages are done. MCP, archiving, production hardening: **not started** |

## What works (verified)

- **Local infrastructure.** Docker Compose runs PostgreSQL 17.6, ClickHouse 25.8 and Kafka 4.1.0
  (KRaft), plus a demo Postgres. Everything is bound to 127.0.0.1 and no image uses `:latest`.
  `just bootstrap` generates every secret into gitignored `.controlhub/`, creates least-privilege
  database users per service, applies migrations, and seeds two tenants (demo + isolation) with
  users and scoped ingestion tokens.
- **Ingest** (Rust, axum and tonic):
  - OTLP/HTTP protobuf for traces and logs (gzip supported), OTLP/gRPC for traces and logs, and
    custom events at `POST /api/v1/events/batch`.
  - JSON OTLP returns 415, and OTLP metrics return 501 (not implemented).
  - Bearer token → SHA-256 lookup. The scope always comes from the token.
  - Server-side redaction, size limits, and a concurrency limit that returns 503 when overloaded.
  - Kafka produce uses `acks=-1`. The HTTP 200 is sent **only after the broker ACK**.
- **Broker outage (tested manually).** With Kafka stopped, clients got 503 after the 15 s ACK
  timeout and the `failed_broker` counter went up. After Kafka restarted, the flow recovered with
  no manual steps.
- **Processor.** Kafka → ClickHouse (`spans`, `logs`, `events`), with offsets in PostgreSQL
  (at-least-once) and undecodable records written to `processing_errors`. Lag is exposed on
  `:9465/metrics`.
- **API** (`/api/v1`, OpenAPI via utoipa):
  - Session login/logout: HttpOnly SameSite=Strict cookie, CSRF double-submit, Origin allowlist,
    Argon2id passwords, login throttling.
  - Read endpoints: projects, services, topology (observed and inferred edges with reasons),
    logs (cursor pagination), traces, trace detail with correlated logs, events, timeline
    (with watermark and completeness), coverage and audit.
  - Every query is tenant-scoped; resources belonging to another tenant return 404.
- **Node SDK** (`@controlhub/sdk-node`): wraps OpenTelemetry with HTTP and pg instrumentation. It
  never collects bodies, full SQL or auth headers. It includes a logger and an events client with
  a bounded queue, retries and counters. It refuses to initialize twice.
- **Demo system.** Gateway → orders → Postgres, a load generator, and fault injection that works
  only on the demo stand and only from loopback.
- **Engine** (Rust, native and WASM in a Web Worker):
  - Timeline index with checkpoints, seek, frames, topology, filters, pick, node metrics and
    compare.
  - Generation-based cancellation of stale seeks, and coalesced requests.
  - Content-hashed WASM artifact with an ABI version check.
- **Web** (Next.js 16):
  - Public pages (/, features, integrations, getting-started, docs) with robots and sitemap.
  - Private `/app`: noindex, no-store, login page, environment picker.
  - Workspace:
    - PixiJS WebGL map with a table fallback, pan/zoom/keyboard controls and a legend.
    - Inspector, including an A/B compare view.
    - Bottom panel: virtualized logs, traces with a waterfall and correlated logs, and events.
    - Timeline in LIVE, PAUSED, REPLAY and HISTORICAL modes: speeds 0.1–10x, step, range picker
      with a 7-day limit, return to live.
    - Completeness badge and an engine diagnostics popover.

### First-working-version acceptance criteria

| # | Criterion | Evidence |
|---|---|---|
| 1 | Bring up dependencies and the app the documented way | Done on this machine by following README. **Not yet repeated on a clean checkout** |
| 2 | Log in to a local project | e2e `helpers.ts` login, integration tests |
| 3 | Run instrumented demo services | `just demo` |
| 4 | Make a real request | load generator, integration test |
| 5 | See its stored trace and logs | integration test "trace detail…", e2e panels test |
| 6 | See real services and their connection on the map | integration topology test, e2e map test |
| 7 | Open a trace and find correlated events | e2e waterfall + correlated logs |
| 8 | Move the timeline to a recorded moment | e2e timeline test (pause/seek) |
| 9 | Replay history and step through events | e2e timeline test (replay, step) |
| 10 | A substantial part of the computation runs in the Rust/WASM engine in a Worker | e2e checks the engine diagnostics |
| 11 | Reopen the page and get the stored history | Opening the page loads the last 15 min from ClickHouse, and HISTORICAL mode loads any range ≤ 7 days. There is no dedicated reload test |
| 12 | Main auth and isolation scenarios | integration tests (401, Origin 403, CSRF, identical 401, cross-tenant 404, token scope), e2e isolation test |

## Checks run in this session (real results)

Machine: Windows 11 (10.0.26200), Intel i9-13980HX, 32 GB RAM, Rust 1.98.1, Node 24.16.0,
pnpm 10.34.6, Docker Desktop.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo check -p controlhub-engine-core -p controlhub-web-engine --target wasm32-unknown-unknown` | pass |
| `cargo test --workspace` | pass: 35 tests (engine unit + proptest, domain, storage, ingest) |
| `pnpm -r --if-present typecheck` | pass (api-client, web-engine, web) |
| `pnpm --filter @controlhub/web lint` | pass (eslint, `no-explicit-any` as error) |
| `pnpm --filter @controlhub/sdk-node test` | pass: 3 tests |
| `node scripts/build-wasm.mjs` | pass: `controlhub_engine.10c7dd9ae476850e.wasm`, 439 KiB |
| `node scripts/codegen.mjs` | pass (ts-rs types, openapi.json, TS client) |
| `pnpm --filter @controlhub/web build` | pass (11 routes + proxy) |
| `node --test "tests/integration/*.test.mjs"` (stack + demo running) | pass: 11/11 |
| `pnpm --filter @controlhub/e2e test` (Playwright, Chromium, production build) | pass: 9/9, including the fault-injection scenario; the timeline test (pause, step to previous/next event, replay, return to live) passed 5/5 with `--repeat-each=5` |
| `just bench` | see below |

Live counters after the run:
- ingest: 2270 requests accepted, 0 `failed_broker`; 19,073 spans, 4,421 logs and 7,569 events
  accepted, 0 rejected.
- ingest: the 2 `rejected_auth` and 1 `rejected_invalid` came from the negative tests.
- processor: lag 0–1 on every partition.

Screenshots from Playwright (gitignored): `test-results/e2e/workspace.png` and
`test-results/e2e/fault-scenario.png`.

Bugs found by the new step-to-event e2e assertion, and fixed:
- The initial load finished with `seek(now)`, which discarded any pause or seek the user had made
  while data was loading. It now re-seeks to the current cursor.
- A live poll already in flight when the user paused could still move the cursor to "now". The
  poll now checks that live mode is still active before it seeks.
- Previous/next event read the throttled React summary, which could still describe an older
  cursor after quick jumps. The step actions now wait for the frame of the latest seek
  (`EngineClient.settledFrame`).

### Benchmark (`just bench`, synthetic data, release build, native, same machine)

| Metric | Value |
|---|---|
| items (spans + logs + events), services | 155,000, 20 |
| `apply_batch` | 68.5 ms |
| first frame (index build) | 147.7 ms |
| random seek + frame | 0.015 ms average over 1000 |
| checkpoints | 498 (interval 512) |
| approximate engine memory | 42.2 MiB |

This is a single run on a laptop, not a statistical benchmark. The WASM build in the browser is
slower; its init time is shown in the diagnostics popover.

## Not run, and why

- **The CI workflow** (`.github/workflows/ci.yml`) has never run: there is no remote and no
  commit yet.
- **A bootstrap from a clean checkout** has not been done. Everything was verified in the working
  tree where it was developed.
- **Linux/macOS** have not been tried; only Windows was used. The scripts are written to be
  cross-platform.
- **Load or soak tests** beyond the demo load (about 5 RPS) have not been run, and there are no
  throughput numbers yet.
- **The broker-outage check** was done by hand, not as an automated test.

## Known limitations

- Live updates use polling (2 s, with a 10 s overlap). There is no WebSocket yet.
- After each live batch the engine rebuilds its timeline index, which costs about 40 ms at demo
  volume. Incremental append is the next engine task.
- The Worker reads timeline metadata from the tail of the response, which depends on the
  serializer's field order (ADR-0009).
- The Pixi map redraws all Graphics on every frame. This is fine for tens of nodes; it is not
  tuned for thousands.
- OTLP metrics return 501. Only traces, logs and custom events are supported.
- A log export that a client retries after a lost ACK is stored twice (ADR-0004). Spans and
  events are deduplicated.
- The OpenTelemetry batch processors inside the Node SDK drop data silently when their queue is
  full; this is not surfaced as a counter yet.
- There is no strict CSP and no TLS in dev. Audit covers successful logins only.
- The processor does not coordinate consumer groups; one instance reads all partitions.
- There is no API or CLI for managing tokens and users; only `bootstrap` creates them.
- The CLI, actions/executor, MCP, archiving and production deployment do not exist.

## Next concrete task

Implement incremental timeline append in `controlhub-engine-core`, so a live batch extends the
index without a full rebuild. Then add WebSocket live delivery with reconnect and a resume token,
replacing polling (stage 2). After that, start the `controlhub` CLI (stage 4). See `TASKS.md`.
