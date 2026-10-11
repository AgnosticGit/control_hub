# Service Metric Cards Implementation Plan
> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an optional persisted System Map layer that expands service cards with accurate CPU, RAM, network, and request-flow metrics, while preserving compact cards when disabled.

**Architecture:** Reuse the existing infrastructure snapshot and current engine frame; derive per-service and per-replica display values in a pure web helper. Keep metric collection and request aggregation in their current paths. Extend the map renderer and backend layout geometry together so expanded cards have safe hit areas and routing clearance. Persist the new layer alongside current project/environment map preferences with backward-compatible defaults.

**Tech Stack:** Rust (`karbird-engine-core`, `karbird-api`), TypeScript/React, PixiJS, Vitest, Playwright, existing `just` workflows.

**Spec:** `docs/superpowers/specs/2026-10-11-service-metric-cards-design.md`

## Global Constraints

- Do not run Git commands or perform Git operations. Leave all changes in the working tree.
- Do not change OTLP contracts, add an endpoint, or add a polling path. Reuse `InfrastructureSnapshot`, `Frame`, and current transport.
- Missing measurements/capacities remain unavailable; never display them as zero or infer a container CPU quota or network bandwidth cap.
- Do not attribute machine disk metrics to a container or service.
- Keep the current compact card appearance when the layer is off; preserve service selection, replica identity, status, external styling, search, and current timeline semantics.
- Write a regression test first for each behavior, then implement the smallest change that makes it pass.
- Treat the screenshot-derived thresholds as contract: green `<70%`, yellow `70%–<90%`, red `≥90%`.

## Review Focus

1. **Replicas with uneven/missing metrics:** aggregate `sum(usage) / sum(known capacity)`; show coverage when any replica lacks required data. Test complete, partial, zero-capacity, and unavailable cases.
2. **CPU denominator:** values are share of host logical CPU capacity, never labeled as container quota utilization. Verify labels and tooltips.
3. **RAM denominator:** use container limit when present, otherwise host memory capacity and label it. No division by zero and no fabricated percentage.
4. **Request-rate window:** edge values in the current frame are calls over the engine rolling window (default 30 seconds). Convert to per-minute consistently; test direction and exact scaling, and do not count external/non-service edges as service-to-service rates unless the spec explicitly allows them.
5. **Expanded geometry:** cards, replicas, search labels, culling, pointer hit tests, edge routing, and machine frames must use compatible dimensions. Bump persisted server-layout version so cached placements built for 172×56 cards are discarded.
6. **Layer lifecycle:** with either Infrastructure or Service metrics enabled, load the existing snapshot; draw machine frames only for Infrastructure. Turning both off stops infrastructure loading. Legacy preference JSON without the new field remains valid and defaults the new layer to off.
7. **Live updates:** new infrastructure snapshots and frames refresh displayed values without a second request loop or stale snapshot when project/environment changes.

---

## Task 1 — Add pure metric derivation and formatting with tests

**Files:**
- Create `apps/web/src/components/workspace/service-card-metrics.ts`
- Create `apps/web/src/components/workspace/service-card-metrics.test.ts`
- Read only as needed: generated `InfrastructureSnapshot` types, `infrastructure.ts`, and `@karbird/web-engine` frame/topology types.

**Step 1: Write failing tests**

Add table-driven tests for:
- aggregating CPU and RAM across replicas by total usage / total known capacity, not average percentages;
- marking aggregate coverage partial when one or more linked replicas lack a metric or denominator;
- absent values and zero denominators returning unavailable, not `0%`/`NaN`/infinity;
- container RAM limit preferred over host total, and host-capacity fallback explicitly identified;
- CPU share described against host logical CPU capacity;
- green/yellow/red boundary behavior at 69.99%, 70%, 89.99%, and 90%;
- byte-rate formatting across B/s, KiB/s, MiB/s and unavailable values;
- RX/TX aggregation over replicas;
- incoming/outgoing request counts derived from topology edge source/target and `frame.edgeIndices`/`EDGE_STRIDE`, with the current rolling-window-to-per-minute conversion;
- exclusion of external nodes and correct orientation (incoming targets service, outgoing originates at service).

Run: `pnpm --filter @karbird/web test -- service-card-metrics.test.ts` — expect failures because the helper is not implemented.

**Step 2: Implement the typed helper**

Export a small typed API, for example:
- `deriveServiceCardMetrics(input): ServiceCardMetrics`
- `formatBytesPerSecond(value): string`
- `utilizationBand(ratio): "healthy" | "warning" | "critical" | "unavailable"`

The input should include a service ID/index, current topology nodes/edges, frame edge buffers and indices, the infrastructure snapshot, and the frame rolling-window duration. Return display-ready numeric values plus denominator/coverage metadata; keep presentation formatting separate where it makes tests clearer. Do not add React or Pixi dependencies to this module.

For traffic rates, use `window_calls * 60 / (window_ns / 1e9)` and guard invalid/nonpositive windows. Confirm the actual frame’s per-edge count semantics before locking the helper API; if the buffer cannot identify the window unambiguously, use the engine’s documented default and add a test for the chosen contract rather than guessing from wall-clock time.

**Step 3: Verify focused tests**

Run: `pnpm --filter @karbird/web test -- service-card-metrics.test.ts` — expect pass.

---

## Task 2 — Add the persisted Service metrics layer and reuse snapshot loading

**Files:**
- `apps/web/src/components/workspace/Workspace.tsx`
- `apps/web/src/components/workspace/MapView.tsx`
- `apps/web/src/components/workspace/useInfrastructure.ts`
- Add/extend focused tests near the extracted map-layer preference parser.

**Step 1: Write failing preference and lifecycle tests**

Test that:
- `serviceMetrics` defaults to `false`;
- old serialized preferences without `serviceMetrics` preserve all four existing settings and parse successfully;
- a saved `serviceMetrics` value round-trips;
- snapshot loading is enabled when Infrastructure **or** Service metrics is enabled and disabled when both are off;
- the renderer can receive a snapshot while infrastructure frames remain hidden when only Service metrics is selected.

Run the focused Vitest files and confirm the new cases fail before implementation.

**Step 2: Implement preference and data-flow changes**

- Add `serviceMetrics: boolean` to `MapLayers` and defaults.
- Make the preference parser backward-compatible: missing new property means `false`; malformed existing required properties still reject the record.
- Add the layer toggle to the existing Map layers menu using the established menu/accessibility pattern.
- Pass the snapshot to `MapView`/`PixiMap` when either visual layer needs it; keep `InfraOverlay` input gated by `infrastructure` alone.
- Change `useInfrastructure` enablement to `layerPrefs.infrastructure || layerPrefs.serviceMetrics` without introducing another fetch, timer, or websocket subscription.
- Ensure snapshot replacement for project/environment changes cannot display stale metrics from the prior scope.

**Step 3: Verify focused tests**

Run focused Vitest tests for layer preferences, `useInfrastructure`, and map-layer controls. Expect all new and existing tests to pass.

---

## Task 3 — Render expanded service and replica cards

**Files:**
- `apps/web/src/renderer/pixi-map.ts`
- Add `apps/web/src/renderer/service-metric-bars.ts` and its test if drawing/math can be cleanly tested outside Pixi.
- Update renderer geometry tests such as `apps/web/src/renderer/edge-route.test.ts` and relevant map tests.

**Step 1: Add failing renderer/helper tests**

Cover:
- compact mode still uses the existing visual content and no metric rows;
- expanded mode draws CPU and RAM segmented bars with the agreed color thresholds and exact value/denominator accessible text;
- network RX/TX and incoming/outgoing request rates are represented without saturation color claims;
- unavailable and partial-coverage states remain visually distinguishable from zero;
- primary and replica cards use the same layout and metrics semantics;
- hit testing and edge routing avoid expanded-card bounds.

Run focused tests; expect the expanded-mode expectations to fail initially.

**Step 2: Implement rendering**

- Add a renderer method such as `setServiceMetrics(snapshot, enabled)` and update it from `MapView` on new snapshots/toggle changes.
- Keep current map cards at 172×56 when the layer is off. In expanded mode use a shared geometry contract of 220×104 for primary and replica cards; if visual inspection shows the content cannot fit accessibly at that size, adjust the value here and update all synchronized geometry in Task 4.
- Render identity/status first, then CPU/RAM segmented bars, then network and request rows. Preserve selected/faded state, service colors, status border, replica highlighting, and keyboard/browser text search behavior.
- Use accessible text or a DOM-backed description/title for exact units and denominators so canvas-only bars are understandable to assistive technologies and browser search remains supported.
- Recompute cards on both frame and infrastructure snapshot updates; avoid needless full Pixi scene reconstruction when only data changes.
- Do not render service metric cards in table-only mode through Pixi; ensure the existing table remains usable and does not claim metrics unless explicitly wired and tested.

**Step 3: Verify focused renderer tests**

Run renderer and helper Vitest tests; expect all compact/expanded, bar, replica, and routing assertions to pass.

---

## Task 4 — Synchronize layout, routing and saved-geometry invalidation

**Files:**
- `crates/karbird-engine-core/src/layout.rs`
- `crates/karbird-engine-core/src/infra.rs`
- `crates/karbird-engine-core/src/server_layout.rs`
- `crates/karbird-api/src/topology.rs`
- `apps/web/src/renderer/infra-layer.ts`
- `apps/web/src/renderer/pixi-map.ts` and geometry tests as required.

**Step 1: Add failing geometry tests**

Test that:
- service and replica bounds match the shared expanded geometry;
- no overlapping service cards occur after layout with expanded dimensions;
- edge corridors and routes do not pass through expanded cards;
- infrastructure machine frames enclose expanded service/replica cards with the required padding;
- old saved server geometry is rejected and rebuilt when the layout version changes.

Run `cargo test -p karbird-engine-core` and focused renderer tests; confirm the new geometry/version tests fail.

**Step 2: Implement shared geometry updates**

- Update engine card half-height and corridor clearance for 220×104 cards (half-height 52; corridor distance must include card half-height plus existing clearance, target 88 pending existing geometric invariants).
- Update infrastructure card half-width from 86 to 110 and matching CHIP dimensions/frame margins.
- Ensure every browser geometry consumer (card drawing, replica drawing, picking, culling, fit bounds, search-label positions, edge route boxes, and overlay chips) reads the same dimensions for the current expanded mode or uses a documented maximum layout bound where dynamic geometry is unsupported.
- Bump `SERVER_LAYOUT_VERSION` from 2 to 3 so persisted geometry generated with old card clearance is invalidated. Update cache-key/version tests without changing layout unrelated behavior.
- Do not change `ENGINE_ABI_VERSION` or typed-array strides; this work does not modify the WASM buffer contract.

**Step 3: Verify geometry tests**

Run `cargo test -p karbird-engine-core`, focused API topology/layout tests, and renderer geometry tests; expect pass.

---

## Task 5 — Add browser coverage and run the full verification set

**Files:**
- `tests/e2e/specs/infrastructure.spec.ts` or new `tests/e2e/specs/service-metric-cards.spec.ts`
- Any selectors/labels needed to make the existing Map layers menu accessible.

**Step 1: Write a failing Playwright scenario**

Use the seeded demo infrastructure and assert through user-visible controls that:
- enabling Service metrics expands service cards and shows available CPU/RAM/network/request values;
- the Infrastructure layer can remain off while service metrics are shown and machine frames remain hidden;
- toggling Service metrics off restores compact cards;
- reloading the same project/environment retains the selected Service metrics layer;
- switching project/environment does not leak old metric values.

Use role/label/text locators and auto-waiting assertions; do not add fixed sleeps.

**Step 2: Run focused browser test**

With `just dev` and `just demo` running, execute the focused Playwright spec and confirm the visible flow passes.

**Step 3: Run required verification**

Run in sequence and report each result:
- `just check`
- `just test`
- `just test-integration`
- `just test-e2e` (with `just dev` and `just demo` running)

If the required local services or dependencies are unavailable, report the exact command and failure reason; do not mark the suite as passed. Do not run Git commands or deploy.

---

## Final acceptance checklist

- The layer is opt-in, persisted per project/environment, and old preferences load with it off.
- Metrics come from the current infrastructure snapshot and engine frame; no new endpoint/polling path exists.
- Resource values are correctly labeled, missing/partial data is honest, and aggregate replica ratios use summed capacity.
- Request direction and rate units are correct for the current rolling window.
- Compact and expanded cards both render correctly; geometry, routes, replicas, search, selection and machine bounds agree.
- Saved topology geometry is invalidated after the geometry version bump.
- Focused tests, `just check`, `just test`, integration, and e2e results are reported accurately.
