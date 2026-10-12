# Timeline WebSocket loading and browser cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Load timeline history over the existing WebSocket, persist complete ranges and arriving signals in a 1 GiB IndexedDB cache, and animate newly visible timeline data.

**Architecture:** The API multiplexes scoped history requests with current live WebSocket traffic and sends one acknowledged chunk at a time. The Worker owns protocol, engine application, reconnect/retry, and IndexedDB; it serves cached coverage first and requests only uncovered ranges. The canvas highlights only newly applied timeline windows.

**Tech Stack:** Rust/Axum/Tokio, ClickHouse storage, TypeScript Web Worker, IndexedDB, WASM timeline engine, Canvas 2D, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-12-timeline-websocket-cache-design.md`

## Global Constraints

- Keep scope derived from the authenticated WebSocket principal and preserve per-environment telemetry authorization.
- Preserve decimal nanosecond strings across API and Worker boundaries; parse with `BigInt` only where arithmetic is required.
- The Worker owns large payload processing and passes raw JSON to WASM; do not parse full history on the UI thread.
- Cache only complete, non-truncated historical coverage; stable-ID deduplication makes retries and live overlap idempotent.
- IndexedDB target limit is exactly 1 GiB; evict the oldest completed ranges first and continue functioning if storage is unavailable.
- On cache database version change, clear all cache records and coverage metadata before reuse.
- Respect `prefers-reduced-motion`; preserve the timeline's existing canvas semantics and live behavior.
- Do not run Git commands or operations; do not commit, push, or create branches.

## Review Focus

1. A reconnect midway through transfer must not mark a partial range complete or lose live messages; retry and deduplicate it.
2. A slow client must not create an unbounded server WebSocket send buffer; ACK-gated chunks enforce backpressure.
3. Quota errors or a 1 GiB eviction must not block engine application or produce false range coverage.
4. Switching accounts/scopes must never expose another account's cached data; cache keys and logout cleanup are scoped.
5. Cache-version changes clear all prior records; reduced-motion users receive no animated reveal.

---

### Task 1: Define and test the history WebSocket control protocol

**Files:**
- Modify: `crates/karbird-api/src/routes/live.rs`
- Test: `crates/karbird-api/src/routes/live.rs`
- Modify: `packages/web-engine/src/protocol.ts`
- Test: `apps/web/src/engine/worker-utils.test.ts` (or a new focused protocol test beside `worker-utils.ts`)

**Interfaces:**
- Client messages: `timeline_load { request_id, from_ns, to_ns, limit }`, `timeline_ack { request_id, sequence }`, `timeline_cancel { request_id }`.
- Server messages: `timeline_chunk { request_id, sequence, from_ns, to_ns, items }`, `timeline_range_complete { request_id, sequence, from_ns, to_ns }`, `timeline_complete { request_id, truncated, completeness }`, and `timeline_error { request_id, message }`.
- Extend `LiveEvent` with typed history chunk/complete/error events and add a Worker request to begin a range load.

- [ ] Test control decoding, decimal nanosecond validation, range ordering, request-ID bounds, ACK/cancel matching, and exact JSON frame shapes.
- [ ] Run `cargo test -p karbird-api routes::live` and the focused web protocol test; confirm new cases fail before implementation.
- [ ] Implement bounded typed control messages and serde server frame variants without changing existing live/edge frames.
- [ ] Re-run focused tests and ensure malformed controls produce request-scoped errors without closing the socket.

### Task 2: Stream scoped history from the API with server-side subdivision and ACK backpressure

**Files:**
- Modify: `crates/karbird-api/src/routes/live.rs`
- Modify: `crates/karbird-api/src/routes/telemetry.rs` only if a shared authorized query helper is needed
- Test: Rust route tests and `tests/integration/live-websocket.test.mjs` (create; no current live WebSocket integration test exists)

**Interfaces:**
- Extract a reusable scoped timeline query function returning the existing `TimelineResponse` DTO.
- History transfer state is keyed by `request_id`; at most one acknowledged chunk is outstanding per socket.

- [ ] Add tests for a complete interval, server subdivision after per-signal truncation, final truncation at configured limits, request cancellation, and concurrent live delivery during history.
- [ ] Run focused API tests to confirm failures before implementation.
- [ ] Query only the authenticated `Scope`; recursively split half-open intervals within depth/request/range budgets, preserving timeline item DTOs and completeness metadata.
- [ ] Send bounded ordered chunks; wait for matching client ACK before sending the next; ignore stale ACKs and cancellation for unrelated request IDs.
- [ ] Integrate history work into the existing socket loop without blocking live fan-out/heartbeats indefinitely; return request-scoped errors on query or budget failure.
- [ ] Extend integration coverage to assert scope isolation, chunk order, ACK backpressure, completion, and continued delivery of live signals.
- [ ] Run `cargo test -p karbird-api` and the focused live integration test.

### Task 3: Add versioned IndexedDB timeline storage with range coverage and 1 GiB eviction

**Files:**
- Create: `apps/web/src/engine/timeline-cache.ts`
- Test: `apps/web/src/engine/timeline-cache.test.ts`
- Modify: `apps/web/package.json` only if a small IndexedDB test shim is needed; avoid adding a dependency if Vitest's browser-compatible fake can be used.

**Interfaces:**
- `openTimelineCache(scopeKey, version)` clears all object stores on version upgrade and returns a cache API.
- `readCoveredRanges(scopeKey, fromNs, toNs)` returns complete cached coverage and missing half-open ranges.
- `commitRange(scopeKey, range, payload, sizeBytes)` atomically stores all received records and complete coverage.
- `putLiveItems(scopeKey, items)` persists stable-ID live signals without claiming whole-range coverage.
- `enforceCacheLimit(maxBytes = 1_073_741_824)` evicts oldest complete ranges first.

- [ ] Test version upgrade clears both items and coverage, scope separation, overlapping range composition, 1 GiB accounting, oldest-first eviction, quota retry/fallback, and incomplete writes never creating coverage.
- [ ] Run the new Vitest file to confirm failure before implementation.
- [ ] Implement IndexedDB object stores and transaction-based APIs; use UTF-8 serialized payload byte counts and `navigator.storage.estimate()` when available.
- [ ] On `QuotaExceededError`, evict oldest completed ranges and retry once; return a non-fatal cache miss if storage still fails.
- [ ] Run focused cache tests and web typecheck.

### Task 4: Move history loading, retries, and cache reads/writes into the Worker

**Files:**
- Modify: `apps/web/src/engine/engine.worker.ts`
- Modify: `apps/web/src/engine/worker-utils.ts`
- Test: `apps/web/src/engine/worker-utils.test.ts` and focused Worker protocol tests
- Modify: `apps/web/src/components/workspace/useWorkspaceEngine.ts`
- Modify: `apps/web/src/components/workspace/timeline/complete-range.ts` and its test if HTTP splitting is removed

**Interfaces:**
- Worker history loads use typed range requests and emit progress/results to the UI.
- Reconnect repeats an incomplete range from its beginning; apply-batch and cache item storage deduplicate stable IDs.
- Full cache coverage produces no history request; mixed coverage requests only missing intervals.

- [ ] Test cached full hit, mixed cached/missing intervals, ACK only after apply and persistence attempt, duplicate chunks on retry, offline/reconnect retry of uncovered intervals only, request cancellation, and incomplete ranges never marked complete.
- [ ] Run focused Worker/cache integration tests to confirm failure before implementation.
- [ ] Have `onLiveMessage` route history frames by `request_id` while still applying ordinary live frames and heartbeats.
- [ ] On `timeline_chunk`, pass raw items to WASM, persist them without blocking live UI, ACK only after successful engine apply and cache transaction outcome; persistence failure remains non-fatal.
- [ ] On `timeline_complete`, atomically commit complete coverage only when server flags permit; on socket close retry after reconnect; avoid retrying canceled/stale generations.
- [ ] Use cache coverage before sending requests; update workspace range/loading state from history progress and remove obsolete recursive HTTP splitting.
- [ ] Run focused web tests, full `pnpm --filter @karbird/web test`, and typecheck.

### Task 5: Animate newly loaded timeline ranges

**Files:**
- Modify: `apps/web/src/components/workspace/timeline/draw.ts`
- Modify: `apps/web/src/components/workspace/timeline/TimelineTracksView.tsx`
- Test: `apps/web/src/components/workspace/timeline/draw.test.ts` plus a focused component test
- Test: `tests/e2e/specs/timeline-cache.spec.ts`

**Interfaces:**
- Timeline view receives bounded loaded intervals and reveal timestamps; drawing derives a short reveal alpha for the affected x-range only.

- [ ] Test reveal clamping to visible bounds, no replay on ordinary redraw/seek, clearing after completion/cancel, and static treatment under reduced motion.
- [ ] Run focused drawing tests to confirm they fail before implementation.
- [ ] Add a requestAnimationFrame-driven, short-lived canvas reveal clipped to newly loaded intervals; avoid animating the full history or redrawing React per signal.
- [ ] Add an accessible loading/progress state and Playwright coverage proving existing data stays visible while new range appears.
- [ ] Run focused unit/component/E2E specs.

### Task 6: Update architecture docs and run required verification

**Files:**
- Modify: `docs/architecture/data-flow.md`
- Modify: `docs/adr/0011-api-tokens-and-live-channel.md` or add a focused ADR only if the protocol choice warrants a permanent decision
- Test: `tests/integration/live-websocket.test.mjs`, `tests/e2e/specs/timeline-cache.spec.ts`

- [ ] Document WebSocket history messages, ACK/retry behavior, cache completeness/version/eviction, and offline limits.
- [ ] Run `just check` and `just test`.
- [ ] With `just dev` and `just demo` running, run `just test-integration` and `just test-e2e`.
- [ ] Report any unavailable or failing suite explicitly; do not claim unrun checks passed.
