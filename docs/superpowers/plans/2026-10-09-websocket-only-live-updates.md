# WebSocket-only Live Updates Implementation Plan

> **For agentic workers:** Implement inline in the current workspace. Do not run Git commands or perform Git operations.

**Goal:** Deliver live telemetry, edge-traffic deltas, and action changes immediately over the existing workspace WebSocket; recover through that socket and never fall back to periodic HTTP polling.

**Architecture:** Keep HTTP for initial/historical and explicit detail queries. Turn WebSocket into an individually framed live channel with resumable recovery. Persist idempotent edge contributions in ClickHouse via the processor, stream stable contribution updates through Kafka, and use PostgreSQL notifications to fan out committed action changes across API instances.

**Tech Stack:** Rust (Axum, sqlx, rdkafka, ClickHouse), TypeScript Worker/React, Vitest, Rust unit/integration tests, existing OpenAPI/codegen workflow.

**Spec:** `docs/superpowers/specs/2026-10-09-websocket-only-live-updates-design.md`

## Global Constraints

- Never use Git commands/operations; no commits, pushes, branches, status or diffs.
- New live telemetry, edge changes, and action changes are sent individually with no timed coalescing.
- HTTP remains for initial snapshots, historical ranges, explicit filters/search, pagination and detail pages.
- Scope and authorization always come from the authenticated principal; client control messages cannot choose a tenant/project/environment.
- Kafka and WebSocket delivery are at-least-once; stable IDs and idempotent storage/client application prevent duplicate visible counts.
- Preserve bounded queues, message limits, session/origin checks, redaction, and existing REST semantics.
- Do not hand-edit lockfiles. Regenerate API contracts if REST DTOs/routes change.

## Review Focus

- Equal timestamps, duplicate Kafka offsets and replay after processor/API restart must not lose or double-count data.
- Socket reconnect and overflow recovery must page to completion without falling back to HTTP polling.
- High-RPS sockets must remain individually framed and must not block Kafka consumers or unrelated scopes.
- Edge subscriptions must be scope-bound and only deliver the selected edge's contributions.
- PostgreSQL action notifications are hints; reconnect must rebuild authoritative current state from PostgreSQL.

## Task 1: Single-message live protocol and WebSocket-only recovery

**Files:** `crates/karbird-api/src/live_hub.rs`, `crates/karbird-api/src/routes/live.rs`, `apps/web/src/engine/protocol.ts`, `apps/web/src/engine/engine.worker.ts`, `apps/web/src/components/workspace/useWorkspaceEngine.ts`, related tests and live-flow docs.

- [x] Add failing protocol and Worker tests for individual `span`, `log`, `event` records and paginated resume messages.
- [x] Change hub delivery so each incoming item is independently enqueued and written; remove server-side 32-item WebSocket arrays.
- [x] Add authenticated WebSocket recovery continuation; replay/catch-up pages are individual messages and cannot require client HTTP timeline recovery.
- [x] Remove live HTTP-poll fallback. On socket failure keep reconnect/backoff and expose stale state; on gap request/continue recovery over the open socket or reconnect cursor.
- [x] Update tests proving no fallback timeline fetch occurs after reconnect failures and replayed IDs are applied only once.
- [x] Run affected API and Worker tests.

## Task 2: Durable, idempotent edge-traffic contributions

**Files:** ClickHouse migration and `crates/karbird-storage/src/ch.rs`; `crates/karbird-processor/src/main.rs` and processor conversion/tests; protocol topic constants; `crates/karbird-api/src/live_consumer.rs` and live-hub tests.

- [x] Add failing tests for deriving one edge contribution from eligible client spans and ignoring non-edge spans / unknown sizes.
- [x] Add a narrow ClickHouse contribution table keyed by scope and stable span ID; persist/retry it before advancing processor offsets.
- [x] Add an internal Kafka traffic-change topic. Processor publishes one stable-ID edge contribution immediately after durable insert; replay of the same source record must publish the same stable change ID.
- [x] Add an API consumer that reads traffic changes and publishes them to scope subscribers without delaying or grouping messages.
- [x] Change the existing edge-traffic query to aggregate durable contribution rows while preserving request/response byte coverage semantics.
- [x] Add integration coverage for selected-edge replay over WebSocket and exact scope/cursor framing; existing integration tests cover persisted query counts/bytes. Execution against this local Kafka stack remains blocked by coordinator errors.
- [x] Run affected crate tests and codegen if a REST response contract changes.

## Task 3: Selected-edge WebSocket subscription and incremental chart

**Files:** WebSocket route/protocol, Worker protocol, `useWorkspaceEngine.ts`, `EdgeInspector.tsx`, edge-traffic helpers/tests, E2E coverage.

- [x] Add failing tests for subscribe/unsubscribe controls, exact scope/edge filtering and change-ID deduplication.
- [x] Add client controls to subscribe to the currently selected edge and unsubscribe on selection/inspector close.
- [x] Return a stable live baseline cursor with the initial HTTP edge-traffic snapshot; subscribe after that cursor to avoid baseline/delta overlap.
- [x] Apply each edge contribution directly to only its affected bucket; no timed buffering or full-range recomputation.
- [x] Reconnect replays durable contributions after the chart cursor over WebSocket; historical mode remains HTTP-only.
- [ ] Add E2E coverage for live bucket growth and no duplicate growth after reconnect/replay (not verified; local E2E run had unrelated failures/timeouts).
- [x] Run affected web tests; browser E2E could not be completed against the current local stack.

## Task 4: Action-plan WebSocket updates

**Files:** PostgreSQL migration/trigger or transactional notifier; `crates/karbird-api` startup/listener/live hub; `crates/karbird-executor` action persistence path if required; workspace action timeline hooks/components and tests.

- [x] Add failing tests for action insert/state transition notification, authorized scope mapping, and reconnect snapshot behavior.
- [x] Publish transactional PostgreSQL notifications for action-plan changes, including changes written by executor processes.
- [x] Run a listener in each API process; resolve notification identifiers to authorized action DTOs and fan them out through the workspace socket.
- [x] Load initial/current action state over HTTP, then apply each socket update by plan ID. Remove timeline action-list polling; retain explicit action detail/history HTTP.
- [x] On reconnect send current action state over the socket before subsequent notifications.
- [x] Test fan-out, duplicates, reconnect and cross-scope isolation; run affected Rust/UI tests.

## Task 5: Full verification and documentation

- [x] Update architecture/data-flow and live protocol docs to match the no-polling, per-message behavior.
- [x] Run `just check` and `just test` (the latter passes Rust/Node tests but exits nonzero at missing host Composer; PHP tests pass via Composer Docker).
- [ ] Run `just test-integration` and `just test-e2e` successfully; prior integration run failed against the local Kafka coordinator and E2E did not complete.
- [x] Review acceptance criteria and record remaining limitations in the SDD ledger.
