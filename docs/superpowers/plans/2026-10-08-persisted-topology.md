# Persisted Server Topology Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Serve a persisted server graph and geometry that open clients adopt automatically without repeated layout resets.

**Architecture:** PostgreSQL stores canonical environment inventory and versioned geometry. A scoped backend builder derives relationships from ClickHouse and computes native Rust layouts. Worker/WASM consumes the complete server revision while continuing time-dependent metrics and Pixi rendering.

**Tech Stack:** Rust, sqlx/PostgreSQL, ClickHouse, axum, WASM, React/Worker, Pixi, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-08-persisted-topology-design.md`

## Global Constraints

- Authenticate and authorize tenant/project/environment before all snapshot and conditional reads.
- Refresh active scopes at ten-second cadence; idle scopes do not aggregate continuously.
- Atomically publish complete snapshots; failed refreshes preserve the last valid revision.
- Node identity is entity ID; edge identity is source/target/relation.
- Metrics alone never change structural revision or geometry.
- Historical membership remains scoped to the exact requested telemetry range.
- Saved live inventory does not silently discard entities at the fifteen-minute boundary.
- Open clients adopt changed graph and placement together and preserve camera/valid selection.
- Nanosecond timestamps cross boundaries as decimal strings.
- No commits, pushes, deployments or modification of unrelated working-tree changes.

## Review Focus

- Concurrent API replicas: only one refresh publishes a given scoped revision.
- Late telemetry: recent historical results must not remain cached indefinitely.
- Resource/replica movement: geometry follows structural host membership, including internal databases.
- Deleted/reclassified edges: selected edge identity includes relation and updates safely.
- Mixed arrival order: render buffers and topology metadata must carry compatible geometry revisions.

### Task 1: Canonical geometry contract and native builder

**Files:** Create `crates/karbird-engine-core/src/server_layout.rs`; modify `lib.rs`, `types.rs`, `layout.rs`, `infra.rs`; tests in the new module and `tests.rs`.

**Interfaces:** Define `SavedTopologyGeometry` with algorithm version, stable-ID node coordinates, source/target/relation corridor points, and optional infrastructure geometry. Export `build_server_geometry(snapshot: &TopologySnapshot, infrastructure: Option<&InfrastructureSnapshot>, previous: Option<&SavedTopologyGeometry>) -> Result<SavedTopologyGeometry, EngineError>`.

- [ ] Write unit/property tests for insertion-order independence, unchanged input, additions preserving existing positions, cycles, long edges, machine replicas and database ownership.
- [ ] Run affected tests and observe regression failures before implementing the behavior.
- [ ] Reuse native layout computation; add anchored incremental placement and collision-free corridors rather than translating a recomputed layout onto old coordinates blindly.
- [ ] Validate finite coordinates, complete node coverage, valid edge references and algorithm version before accepting geometry.
- [ ] Run `cargo test -p karbird-engine-core`.

### Task 2: Durable scoped graph storage

**Files:** Create `infra/migrations/postgres/0013_topology_snapshots.sql`, `crates/karbird-storage/src/topology.rs`; modify storage module exports; integration tests in `tests/integration/topology-persistence.test.mjs`.

**Interfaces:** Store canonical graph JSON, geometry JSON, structural fingerprint, revision, freshness/source watermark and schema version under tenant/project/environment. Use separate bounded historical cache rows keyed by scope/range/version/watermark. Provide read, refresh-lease, atomic publication and pruning methods through `PgStore`.

- [ ] Add storage tests for isolation, stale lease takeover, concurrent lease attempts, atomic publication, restart persistence and bounded cache eviction.
- [ ] Implement migration with scope foreign keys and cascading project/environment deletion, JSON size bounds and indexes for freshness/pruning.
- [ ] Implement transactional lease/publication with fencing token; a timed-out old builder cannot overwrite a newer result.
- [ ] Record first/last seen inventory and explicit inactivity; preserve stable IDs across refreshes.
- [ ] Run storage unit tests and persistence integration tests against migrated local PostgreSQL.

### Task 3: Backend refresh and topology reads

**Files:** Create `crates/karbird-api/src/topology.rs`; modify `routes/telemetry.rs`, `routes/mod.rs`, `state.rs`, `main.rs`; update topology integration tests.

**Interfaces:** Move relationship aggregation into a graph builder; introduce a bounded refresh coordinator in `AppState`. Extend `TopologyResponse` with revision, versioned saved geometry and freshness. Support authenticated ETag/If-None-Match reads for structural snapshots without discarding requested-range metrics.

- [ ] Add handler/integration tests for 401/403, tenant isolation, conditional authorization, first build, immediate saved read, refresh failure, identical graph revision and structural change.
- [ ] Register active scopes with bounded capacity and idle expiry; build on a ten-second interval with limited concurrency and cross-process storage lease.
- [ ] Return saved geometry immediately; perform refresh in background. Await one bounded build only when no valid snapshot exists.
- [ ] Canonicalize fingerprints by stable identity; separate requested-range evidence from durable inventory. Track processor watermarks and refresh recent historical cache on late telemetry.
- [ ] Count/log failed refreshes, lease contention and stale reads; shutdown cancels background work safely.
- [ ] Run `cargo test -p karbird-api` and relevant real-stack integration tests.

### Task 4: Consume complete saved revisions in WASM and Worker

**Files:** Modify `crates/karbird-engine-core/src/lib.rs`, `types.rs`, `infra.rs`, `crates/karbird-web-engine/src/lib.rs`, `apps/web/src/engine/engine.worker.ts`, `client.ts`, `packages/web-engine/src/protocol.ts`, `apps/web/src/components/workspace/useWorkspaceEngine.ts`.

**Interfaces:** Load topology membership and `SavedTopologyGeometry` as one server revision. Matching revisions update evidence without rebuilding indices/layout. Timeline-derived unknown identities request backend refresh instead of changing workspace canonical geometry. Keep an explicit local-layout mode for public demo and standalone engine users.

- [ ] Write tests for repeated snapshots retaining indices/revision, metric-only refresh, incompatible geometry rejection, unknown timeline service discovery, revision arrival ordering and stale requests.
- [ ] Apply a changed server revision atomically, remapping reducer references through stable entity IDs and invalidating only affected caches.
- [ ] Use saved infrastructure placements; current metrics update decorations without recomputing geometry. Problem/search filters hide or dim nodes without changing saved positions.
- [ ] Deliver topology metadata before the corresponding render frame; discard obsolete topology responses rather than overwriting newer revisions.
- [ ] Add conditional polling and backend refresh hints with bounded coalescing. Keep ten-second graph freshness for open clients.
- [ ] Run engine unit/property tests, Worker/client Vitest tests; regenerate `just codegen` and rebuild `just wasm`.

### Task 5: Stable Pixi rendering and client update regression

**Files:** Modify `apps/web/src/renderer/pixi-map.ts`, `apps/web/src/components/workspace/MapView.tsx`, `edge-traffic.ts`; add focused unit tests and `tests/e2e/specs/topology-persistence.spec.ts`.

- [ ] Write meaningful regression tests for unchanged revision keeping routes/camera, matching topology/frame geometry, relation-aware selected edges and deleted selected elements.
- [ ] Reuse existing static labels/routes when geometry is unchanged; draw changed revision together without temporary old-coordinate routes.
- [ ] Keep camera on server updates and layer toggles after initial fit; preserve selection by stable identity across reindexing.
- [ ] Browser-test reload persistence, multiple clients sharing geometry, live backend topology change reaching both clients, new service placement, Infrastructure replica movement and resize/filter stability.
- [ ] Confirm table fallback consumes the same canonical graph without WebGL.

### Task 6: Operations, contracts and final verification

**Files:** Add an ADR and update `docs/adr/README.md`; update generated OpenAPI/TS/WASM artifacts and scoped integration/MCP tests.

- [ ] Document schema/algorithm version upgrades, cold-start behavior, freshness, graph inactivity, historical cache bounds and refresh observability.
- [ ] Verify MCP topology exposes the same persisted canonical relationships and authorized scope.
- [ ] Run `just check`, `just test`, `just test-integration`, `just test-e2e` with the dev/demo stack running. Explicitly report unavailable PHP tooling and any unrelated failures.
- [ ] Inspect final diff and codegen repeatability; list changed files and any remaining limitations without committing or pushing.

## Execution

Native execution used the existing checkout on `feature/persisted-topology`, preserving prior resize changes. The detailed execution and verification record is `.superpowers/sdd/2026-10-08-persisted-topology/progress.md`.

Independent review identified five Important findings; all five were addressed with regressions: queued cold publication, late historical adoption, backward corridors, graph-only infrastructure anchoring, and stale CPU labels.

Implementation decisions refine the original checklist: scoped ten-second historical revalidation replaces global processor-watermark keys; fenced PostgreSQL leases replace advisory locks; JSON documents contain graph, geometry and version together; refresh failures are logged. History and selected-time placement variants share a 64-entry cache. Two shared CPU permits remain held by blocking jobs through cancellation.

Primary changed modules:

- Backend reads/builds: `crates/karbird-api/src/topology.rs`, `routes/telemetry.rs`, `routes/infrastructure.rs`; table grants in `bootstrap.rs`.
- Persistence: `crates/karbird-storage/src/topology.rs`, module export, `infra/migrations/postgres/0013_topology_snapshots.sql`.
- Native/WASM ownership: `crates/karbird-engine-core/src/server_layout.rs`, `lib.rs`, `infra.rs`, `types.rs`, `tests.rs`.
- Browser adoption: `engine.worker.ts`, `topology-cache.ts`, `useWorkspaceEngine.ts`, `MapView.tsx`, `NodeTable.tsx`, `InfraOverlay.tsx`, `useInfrastructure.ts`, `infrastructure-refresh.ts`, `pixi-map.ts`, `fit.ts`, `infra-layer.ts`, edge path/routing/selection helpers and inspector.
- Contracts: Worker protocol, generated EngineConfig, OpenAPI and API client; rebuilt WASM artifact.
- Regression suites: native tests; cache/table/poll/edge Vitest tests; topology persistence/MCP/API/HTTP integration; topology persistence/infrastructure/resize Playwright tests.
- Documentation: ADR-0022 and its index, this plan and its approved specification.

Pre-existing resize edits in the timeline components and their tests were retained. No commits, pushes or deployments were made.

## Delivered verification

- `just check` passed; contracts regenerated without drift and WASM rebuilt.
- Rust: engine 168 and API 207 tests passed; the workspace Rust run also passed.
- Web: 442 tests passed; Node SDK 30 and scripts/demo 98 passed.
- Full integration: 180 passed, zero failed, one deliberately separate broker-outage test skipped.
- Eight affected browser scenarios passed across the final targeted runs, including live two-client adoption, late history, edge traffic, infrastructure/replicas/picking, resize and timeline neighbours.
- The full baseline browser run had 116 passes, 28 failures, one skip and six serial dependents not run. Relevant map failures were fixed and rerun; unrelated outdated account/admin/auth/navigation/signup/role expectations remain. PHP tests could not run because Composer is absent. Neither suite is reported as fully green.
