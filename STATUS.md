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

### Bugs found by running every `just` command (2026-09-29)
- **`just dev` failed with EBUSY** when an earlier run's children outlived a force-killed
  supervisor. `just dev`/`just demo` now write PID files to `.controlhub/run/`, refuse to start
  twice, and the new `just stop` kills recorded processes with their children (containers stay up).
  The web app is started as `node …/next dev` instead of through the `pnpm`/`cmd` wrapper, so the
  recorded PID is `next` itself and its server child is stopped too.
- **Completeness watermark lagged ~23 min while `processing_caught_up=true`.** An idle Kafka
  partition never updated its offset row, and the watermark is the minimum over partitions. A
  caught-up idle partition now refreshes its row every 5 s (lag after the fix: 1.4 s).
- **`just cli` lost exit codes**: PowerShell `-Command` turned the documented exit code 2 of
  `apply` without `--yes` into 1. The recipe now ends with `exit $LASTEXITCODE`.
- **CSP e2e specs failed under `just dev`**: `next dev` needs `'unsafe-eval'` (added outside
  production by `csp.ts`), and the specs only knew the production policy. They now detect the
  Next.js dev overlay and require no `'unsafe-eval'` only in production builds.

### Honest request animation, 0.001× replay, redesign (2026-09-29)
- **The dots on the map used to be decorative.** A global 1.6 s phase moved a dot on every busy
  edge at once, so gateway→orders and orders→Postgres always looked simultaneous. The engine
  now builds request hops from recorded spans:
  - A call hop pairs the caller's client span with the callee's server span and keeps four real
    timestamps: request leaves, reaches the callee, the callee answers, the answer is back.
  - A leaf hop is a client span to a component without instrumentation. Only its start and end
    are known, so the dot stays at the target for the whole call.
  - Clock skew is clamped into order and flagged `HOP_CLAMPED`.
  - The frame carries `hopData` (edge, t0..t3 relative to frame time, flags), capped at 3000
    hops. The cap is reported as `hops_shown / hops_in_window`.
  - The renderer moves each dot only between its own timestamps.
    - Live runs 8 s behind the wall clock, because SDKs export spans in 5 s batches; the map says
      so.
    - Replay advances at the chosen speed; paused is frozen.
    - Engine ABI is now 2.
- **Replay speeds** are 0.001×, 0.01×, 0.1×, 0.5×, 1×, 2× and 10×. 0.001× plays 1 ms of recorded
  time per second, and below 0.1× the clock shows microseconds. Measured: 3 s wall time moved the
  cursor 3.08 ms.
- **Redesign after the reference screenshot, visual only.** It adds a dark theme, a sidebar
  (project, environment, Overview/Actions/Tokens, system status), a top bar (search, mode, the
  1h/6h/24h presets that already existed), and KPI cards built only from existing numbers
  (services, requests/min from the 30 s window, error rate, active requests).
  - The map now shows card nodes, curved glowing edges and arrows.
  - Inspector, signals, timeline, Actions, Tokens, login and public pages are restyled.
  - No new features were added.

### Layered, zoomable timeline (2026-09-29)
- The bottom timeline is now a layered track view, like a video editor. It replaces the plain
  slider. Spec: `docs/superpowers/specs/2026-09-29-layered-timeline-design.md`.
- The engine computes `timeline_tracks(from, to, buckets, clip_cap)` over the loaded data. The
  map filters (search, hide inferred) apply to every track.
- Track rows:
  - Actions: plan bars from `/actions`, coloured by state.
  - Events: counts per bucket, plus markers for rare event types (not `request.*`).
  - Errors: failed request spans and logs at severity ERROR or above.
  - Requests: server, consumer and root spans, counted by start time.
  - Latency p95: nearest-rank per bucket, by end time. Empty buckets are gaps, not zeros.
  - One row per service: a heat strip, or individual span clips (up to 3 depth lanes) once no
    more than 400 spans are in view.
- Zoom: the wheel zooms around the pointer, and there are "Zoom in time" / "Zoom out time"
  buttons. Shift+wheel or drag pans; a click moves the cursor. "Whole range" resets the view.
  The minimum view is 1 ms.
- While playing, the view pages so the playhead stays inside it. (The overview strip and the
  separate "Selected time" slider were later removed; see "Workspace UX refactor" below.)
- Deviation from the spec: JSON results instead of typed arrays, and a full span scan per call
  instead of a start-sorted index (noted in the spec).

### Map line styles (2026-09-29)
- A "Line style" switch next to the map zoom buttons offers three styles:
  - Curved: the previous bowed lines.
  - Square: right angles with rounded corners.
  - Metro: straight runs with a 45° diagonal, the extra option.
- The choice is kept in `localStorage` (`controlhub.map.edgeStyle`).
- The geometry is a pure module, `apps/web/src/renderer/edge-path.ts`, with vitest tests. Every
  style is a polyline, and request dots move along it by arc length. The dots follow the drawn
  line in every style and still keep only the recorded timestamps.
- Opposite directions between the same cards get separate lanes (5 px) in the square and metro
  styles.
- With the original three demo services on one row, square and metro looked straight. The demo
  now has seven services (see below), so the bends are visible on the map itself.

### Workspace UX refactor, exact dots, larger demo (2026-09-29)
- **Timeline without the slider.** The bottom range slider and the overview strip are gone.
  The ruler itself is the cursor: drag it (or drag across the rows) to move through time, and
  the cursor follows the pointer continuously. Shift+drag or the middle button pans. Ctrl/⌘+wheel
  (or any wheel over the ruler) zooms, Shift+wheel pans, and the plain wheel scrolls the rows
  vertically. The ruler is a `role="slider"` "Selected time" with Left/Right (Shift ×10),
  PageUp/PageDown and Home/End. The zoom buttons centre the window on the cursor.
- **Timeline is a compact, scrollable dock.** One header row holds mode, transport, speed,
  live, cursor clock, the `active · req · err` summary, Mark A and Range… (now a popover). The
  ruler is sticky and the tracks scroll vertically. The dock height is resizable by dragging its
  top edge or with the arrow keys (separator "Resize bottom dock", 150–640 px, default 220 px,
  remembered in `localStorage`).
- **Page layout.** The map is the main area. KPIs are one compact row. Logs/Traces/Events moved
  from a bottom panel into a right-hand side panel with tabs "Details" (Inspector) and "Signals".
  Selecting a component (or running Compare) switches to Details; Signals is the default. The
  side panel width is resizable (320–760 px, default 400 px, remembered). The logs, traces and
  events views adapt to the narrow panel with container queries: the trace list stacks above the
  waterfall, and the event attributes column hides. The map legend wraps instead of covering the
  zoom and line-style buttons.
- **Why dots were not in the middle of a request (root cause).** OpenTelemetry JS 2.x takes a
  span's start from `Date.now()` (1 ms resolution) and adds a precise duration for the end.
  Every span of a fast request therefore appeared to start in the same millisecond, and the
  hop timestamps were wrong by up to 1 ms on 2–5 ms calls. `@controlhub/sdk-node` now adds a
  `PreciseStartSpanProcessor` that restamps the start from `performance.timeOrigin +
  performance.now()` (sub-millisecond). An explicit start time from the caller is kept. Tests:
  `packages/sdk-node/test/precise-start.test.js`.
- **Dot model.** The user chose "progress": a dot moves from caller to callee over the whole call,
  `(now − t0) / (t3 − t0)` of the caller's client span. At the middle of a call the dot is in
  the middle of the line. The map note says so ("dot position = share of the call elapsed").
- **Clock skew between services.** Measured on this machine, Node processes disagree by up to
  about 1 ms: `performance.timeOrigin` spreads 2.7 ms and `Date.now()` 0.8 ms across 6 processes,
  checked against the system-wide `process.hrtime`. On live data, 172 of 1,604 cross-service
  server spans started before their caller's client span and 931 ended after it. The engine now
  corrects this for display only (new `skew.rs`), as trace viewers do. A callee whose entry span
  does not fit inside the caller's span is shifted, with its whole in-service subtree, to sit
  centred in it. Shifts accumulate down the chain. The correction is applied to map hops and to
  timeline clips, so both show the same times. Stored data is unchanged.
- **Demo: 7 services.** gateway, auth, catalog, orders, inventory, payments and notifications,
  plus Postgres and an uninstrumented payment-provider stand-in:
  - gateway checks every `/api/*` request with auth.
  - `GET /api/catalog/:id` goes to catalog, which calls inventory.
  - `POST /api/orders` goes to orders, which reserves in inventory, charges through payments
    (which calls the provider), writes to Postgres and notifies.
  - Load mix: 15% POST orders, 30% catalog reads, the rest order reads.
  - `just demo-fault` accepts every service.

### Map routing, service focus, replay range (2026-09-29)
Spec: `docs/superpowers/specs/2026-09-29-map-focus-range-design.md`.
- **Lines no longer cross or run under cards.** The engine layout (`layout.rs`) is now layered
  (Sugiyama-style):
  - Longest-path columns, with a corridor item for every column that a long edge skips.
  - Barycenter sweeps plus neighbour swaps to minimise crossings.
  - Heights set by isotonic regression (PAV) with minimum gaps.
  - Each edge carries its corridor waypoints (`TopologyViewEdge.points`).

  The renderer (`apps/web/src/renderer/edge-route.ts`, pure, vitest) routes every edge through
  these corridors in all three line styles:
  - Separate ports along the card sides, ordered by the other end.
  - Separate lanes for vertical runs that share a gap.
  - Routes are recomputed only when the topology or the style changes.

  Tests:
  - Engine: no crossings, corridors and cards never overlap on the demo topology, a proptest on
    random graphs, determinism.
  - Web: no route point lies inside a foreign card, in any style.
- **Changed invariant.** Cards may move once when a new component or connection appears, because
  the layout is recomputed. The old test "layout is stable when new nodes appear" was replaced by
  "layout does not move when only items change".
- **Switching services off.** Every service row on the timeline has a switch. There are also
  "All on" / "All off" buttons and a "Show only this service" button, shown on hover. Switched-off
  services are dimmed, not hidden:
  - The timeline row fades, and the service is left out of the Requests, Errors, Latency and
    Events rows.
  - On the map its card, edges and request dots fade (`Filters.muted_services`, `FLAG_DIMMED`,
    `HOP_DIMMED`).
  - A dependency (for example Postgres) fades only when every caller is switched off.

  To look at one interaction, switch everything off and turn two services back on.
- **Replay range.** Dragging across the tracks selects a range. A click on the tracks, or a drag
  on the ruler, still moves the cursor.
  - The view zooms to the range, and the time outside it is shaded.
  - The range edges can be dragged.
  - A header chip shows the range; its ✕ (or Esc) clears it and shows the whole timeline again.
  - Play loops inside the range until it is cleared. Picking a range leaves live mode.
- **Tooltip.** The timeline tooltip is centred directly above the pointer and clamped to the
  window edges. It flips below the pointer near the top of the window.
- **PAUSED** is now a neutral grey instead of the amber used for degraded services.

### Test coverage pass (2026-09-29)
Every crate and package now has unit tests, and the e2e and integration suites cover every page,
user flow and API route found. Test counts:

| Area | Before | After |
|---|---|---|
| Rust (`cargo test --workspace`) | 83 | 514 (proptests included) |
| Web vitest | 44 | 165 |
| SDK (`node --test`) | 11 | 18 |
| Demo system + scripts (`node --test`, now part of `just test`) | 0 | 24 |
| Integration | 32 | 71 |
| e2e (Playwright) | 24 | 91 |

Real bugs these tests found, all fixed and covered by a regression test:
- **Ingestion redaction:**
  - A token after two spaces or a tab (`Bearer  <token>`) was not masked.
  - A sensitive fragment past the 128-byte key limit escaped redaction; custom events had the
    same leak.
  - Nested secrets and payloads in custom events were stored verbatim.
  - URL query strings (`url.full`, `url.query`, `http.url`, `http.target`) were kept. Their
    query part is now `[REDACTED]`.
- **Ingestion counting and limits:**
  - Attributes over the per-item limit were dropped without a counter. The new counter is
    `controlhub_ingest_attributes_truncated_total`.
  - The queued-bytes gauge leaked when there were no partitions.
  - Content types were case-sensitive, and for events any `application/json*` prefix was
    accepted as JSON.
- **Startup:** it could hang forever when Kafka was down, because rskafka's first connection had
  no deadline.
- **API:** the 7-day range check could overflow, so `from=i64::MIN&to=i64::MAX` passed in release
  builds. The ClickHouse span lookback had the same kind of overflow.
- **Engine:**
  - Timestamps near `i64` limits overflowed in several places.
  - A rejected batch was half-applied.
  - The `only_problems` filter kept edges to hidden nodes.
  - A negative rolling window gave negative counts.
- **Demo tools:** a non-numeric `DEMO_RPS` flooded the gateway, and a typo in a
  `demo-fault --latency` value silently cleared the fault.
- **Web:**
  - The workspace page had no `<h1>`; the "System Map" title is now one.
  - Private `/app` pages now send `Cache-Control: no-store` from `proxy.ts`.

  `next dev` still forces `no-cache, must-revalidate` on rendered pages, so that e2e test skips
  itself against a dev server. Against `next start` every `/app` page, logged in or not,
  returned `no-store` (checked by hand on port 3001).

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

Second pass, every `just` recipe against `just dev` (web in `next dev` mode) + `just demo`:

| Command | Result |
|---|---|
| `just`, `just doctor`, `just down`, `just reset` (answered "no": aborted, volumes kept), `just bootstrap`, `just wasm`, `just codegen`, `just build` | pass |
| `just check`, `just test` | pass (re-run after the fixes above) |
| `just dev`, `just dev-backend`, `just demo`, `just stop` | pass: ports up; a second `just dev` is refused; `just stop` stopped 8 (dev+demo) and 4 (dev-backend) processes and freed every port |
| `just demo-fault orders --latency 100 --error-rate 0.2`, `--clear` | pass (read back) |
| `just cli doctor/discover/plan/apply/apply --yes/verify/context/mcp` on a temp copy of the fixture | pass; `apply` without `--yes` exits 2 |
| `just test-integration` | pass: 31/31, 1 skipped (broker outage) |
| `just test-e2e` | 14/16 on first run (CSP specs, fixed above), then 15/16: the actions spec was refused by the policy rate limit (`30 plan(s) in the last hour`) after the earlier runs; after a slot freed, `specs/actions.spec.ts` passed 2/2 and `specs/csp.spec.ts` 2/2 |
| `just test-broker-outage` | pass: 1/1 |
| `just bench` | 155k items, first frame 113.3 ms, seek 0.011 ms, live batch + frame 4.6 ms, ~44 MiB |

Third pass, after honest hops and the redesign, against `just dev` + `just demo`:

| Command | Result |
|---|---|
| `just wasm`, `just codegen` | pass; ABI 2, WASM 474 KiB; codegen shows no drift |
| `just check` | pass |
| `just test` | pass: engine-core 22 tests (6 new hop tests), web vitest 6/6 (hop math), all other crates and SDK tests pass |
| `just test-e2e` | pass: **16/16** with the new layout (all accessible labels kept) |
| Visual check (Playwright screenshots, 1600×960) | live and replay layouts render. At 0.001× a request dot waits on the orders card while gateway→orders is lit, then disappears when the recorded call ends |
| `just test-integration`, `just bench` | not re-run: ingest, processor and API are unchanged since the second pass |

Fourth pass, after the layered timeline, against `just dev` + `just demo`:

| Command | Result |
|---|---|
| `just wasm`, `just codegen` | pass; WASM 529 KiB; new generated types `TimelineTracks`, `ServiceTrack`, `SpanClip`, `EventMarker` |
| `just check` | pass (after collapsing three `if let` for clippy) |
| `just test` | pass: engine-core 28 tests (6 new track tests), web vitest 14/14 (8 new scale tests), other crates and SDK pass |
| `just test-e2e` | pass: **16/16**; then `specs/workspace.spec.ts` with the new "layered timeline shows tracks per service and zooms" test: 7/7 |
| Visual check (temporary Playwright screenshots at 1600×1000, spec deleted afterwards) | whole range at 15 min, 3.5 s and 220 ms views render; at 220 ms the service rows show individual span clips and the hover tooltip; no console errors |
| `just test-integration`, `just bench` | not run: ingest, processor and API are unchanged; the bench does not cover `timeline_tracks` |

Fifth pass, map line styles (web only), against `just dev` + `just demo`:

| Command | Result |
|---|---|
| `just check` | pass |
| `just test` | pass: web vitest 24/24 (10 new edge-path tests), Rust and SDK unchanged and passing |
| `just test-e2e` | pass: **18/18**, including the new "map line style can be switched and is remembered" test |
| Visual check (temporary Playwright screenshots and an SVG preview of a branching graph; both deleted) | all three styles render, and the choice survives a reload; no console errors |

Sixth pass, UX refactor + precise span starts + clock-skew correction + 7-service demo, against
`just dev` + `just demo` (restarted with the new SDK):

| Command | Result |
|---|---|
| `just wasm` | pass; WASM 534 KiB. Engine types unchanged, so no codegen output changes |
| `just check` | pass (after `cargo fmt`) |
| `just test` | pass: engine-core 33 tests (5 new skew tests), SDK tests pass (8 new precise-start tests), web vitest 25/25 (new `callProgress` and `zoomCentered` tests), other crates unchanged and passing |
| `just test-e2e` | pass: **20/20**, including the new "timeline cursor is dragged along the ruler and the dock is resizable" and "selecting a component opens its details in the side panel" tests |
| `just test-integration` | first run 30/32 with 1 failure: the trace test assumed the first multi-service trace contains orders, but the new demo also has gateway→auth/catalog traces. After fixing the test to pick a trace containing orders: **31 pass, 1 skipped** (broker outage), 0 fail |
| ClickHouse check on live data | all 7 services report spans; span starts are sub-millisecond and ordered within a process. Across processes, the skew described above remains in the stored data (by design; corrected at display) |
| Visual check (temporary Playwright screenshots at 1600×900 and 1366×768; spec deleted) | map, side panel (Signals and Traces stacked), dock with drag-scrub and a 220 ms zoom render correctly at both sizes. The legend now wraps instead of covering the line-style buttons |

Seventh pass: map routing, service focus and replay range, against `just dev` + `just demo`.

| Command | Result |
|---|---|
| `just codegen`, `just wasm` | pass. New fields `Filters.muted_services` and `TopologyViewEdge.points`. WASM 588 KiB |
| `just check` | pass (after `cargo fmt`) |
| `just test` | pass: engine-core 41 tests (new layout tests incl. a proptest, and mute tests), web vitest 44/44 (new `edge-route` and `range` tests), other crates and SDK pass |
| `just test-e2e` | first run 23/24: the new range test used a label that also matched the ✕ button. After fixing the test: **24/24**, including 4 new tests (service switches with All on/off and show-only, replay range with looping play and ✕ reset, a click on the tracks plus the tooltip above the pointer at the right edge, the PAUSED colour) |
| `just test-integration` | **31 pass, 1 skipped** (broker outage), 0 fail |
| Visual check (temporary Playwright screenshots at 1600×1000; spec deleted) | curved, square and metro: no crossings, and no line under a card. Gateway + orders focused with the rest dimmed on the map and the timeline. Range chip, shading and the grey PAUSED badge render |

Eighth pass: test coverage, against `just dev` + `just demo`.

| Command | Result |
|---|---|
| `just wasm`, `just codegen` | pass (WASM 588 KiB, no contract drift) |
| `just check` | pass (after two clippy fixes in the new redaction code) |
| `just test` | pass: **514** Rust tests, web vitest **165/165**, SDK **18/18**, demo system + scripts **24/24** (`node --test`) |
| `just test-integration` | first run 69 pass, 1 fail, 1 skipped (broker outage). The failure was `actions.test.mjs` refused by the policy rate limit (`30 plan(s) in the last hour`). After the window freed, `node --test tests/integration/actions.test.mjs` **5/5** (incl. the DB restart) |
| `just test-e2e` | first run 88 pass, 1 fail, 2 skipped. The failure was the actions spec (same rate limit); one skip is the dependent reject test, the other is the Cache-Control test, which skips on `next dev`. After the window freed, `specs/actions.spec.ts` **3/3** |
| Cache-Control on `next start` (by hand, port 3001 with a session from port 3000) | every `/app` page returns `no-store` |

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
- **A clean orphan scenario for `just stop`** (supervisor killed while its children survive) could
  not be reproduced here: the tool shell killed the whole process tree. `just stop` was verified
  against running supervisors.

## Known limitations

- Actions exist only for the demo system (3 actions). Verification is statistical over 15 s: it
  shows the service receives traffic with a bounded error ratio, not causality.
- Policy limits are fixed in code (30 plans/hour per environment, one open plan per target,
  30 min decision window, 10 min start window). There is no per-tenant configuration. Running
  `just test-integration` and `just test-e2e` several times within an hour exhausts the demo
  environment's limit, and the actions e2e spec then fails until the window frees.
- After `just stop`, the stopped `just dev`/`just demo` terminals report exit code 1 (the
  supervisor sees its children killed).
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
- At 1× a demo request lasts about 2–5 ms, shorter than one display frame, so the honest dots are
  barely visible live. Slow replay (0.01× or 0.001×) is the way to watch a request travel.
  Live dots run 8 s behind real time.
- Hops are drawn only on edges visible in the frame. Messaging hops (consumer spans) are not
  exercised by the demo.
- Timeline tracks show only the loaded range. When the load is truncated, the tracks warn
  about it. The last few seconds of live data are often empty because SDKs export in batches.
  `timeline_tracks` scans every loaded span per call and has no benchmark yet.
- Clock-skew correction is a display heuristic (centring). It is applied to map hops and
  timeline clips but not yet to the trace waterfall in the Signals panel, which still shows the
  raw per-service times. Services instrumented with plain OpenTelemetry JS instead of
  `@controlhub/sdk-node` still report span starts with 1 ms resolution.
- `@controlhub/sdk-node` and the CLI are not published. Projects reference them from a checkout.
- Map routing guarantees are for left-to-right edges. Cycles and edges within one column fall
  back to a direct line, which can still cross others. Large graphs (over 400 layout items) skip
  crossing minimisation. At low zoom, parallel lanes in one gap are only a few pixels apart.
- Switched-off services and the replay range are not remembered across reloads. The loop runs
  only inside the loaded range; loading another range or returning to live clears it.

## Next concrete task

Commit and push (user decision), get CI green, then verify `just bootstrap` from a clean checkout
on Linux.
After that: an automated executor-loss test, and server-side filters on the live channel. See
`TASKS.md`.
