# Month-scale timeline implementation plan

## Goal and boundaries

Implement the approved design in `docs/superpowers/specs/2026-10-09-month-scale-timeline-design.md`: a 30-day aggregated overview, progressive raw detail loading at closer zoom, edge-triggered adjacent loads while panning, and a timeline-native loading animation. Preserve the current live stream and seven-day maximum for raw timeline requests. Do not change telemetry ingestion or retention.

No Git operations are part of this work, per the repository instructions.

## Files and responsibilities

- `crates/karbird-storage/src/ch.rs`: bounded ClickHouse aggregate queries for span, log, event, and machine series.
- `crates/karbird-api/src/routes/telemetry.rs`, `crates/karbird-api/src/main.rs`, `crates/karbird-api/src/openapi.rs`: scoped 30-day summary handler and route documentation.
- `crates/karbird-api/src/routes/telemetry.rs` tests and `tests/integration/`: aggregation correctness and tenant/project/environment isolation.
- `apps/web/src/components/workspace/useWorkspaceEngine.ts`: summary/detail load orchestration, expanded loaded bounds, deduped chunk requests and generation checks.
- `apps/web/src/engine/engine.worker.ts` and worker protocol: apply aggregate summaries and raw chunks without stale responses replacing current state.
- `apps/web/src/components/workspace/timeline/scale.ts` and tests: 30-day zoom ceiling and adjacent chunk window calculations.
- `apps/web/src/components/workspace/timeline/TimelineTracksView.tsx` and timeline styles: adaptive overview/detail rendering, edge-load state and motion-aware animation.
- `tests/e2e/specs/`: user-visible zoom, pan, progressive loading and reduced-motion coverage.
- Generated OpenAPI/TypeScript artifacts: regenerate with `just codegen`; do not edit generated files manually.

## Tasks

### 1. Define and test aggregate response and range behavior

- Add a typed summary response with string nanosecond boundaries, bucket width/count, request and error counts, p95 latency, per-service span counts, error-log counts, event counts, and host CPU/RAM samples.
- Add a range resolver capped at 30 days and validate `from < to`, bucket count clamping, and empty ranges.
- Add handler tests proving scope comes from the authenticated principal and data from another tenant/project/environment is not returned.
- Run: `cargo test -p karbird-api timeline_summary`.

### 2. Implement bounded ClickHouse aggregation

- Add storage queries that bucket by integer nanosecond intervals and return no more than 2,000 buckets per requested series.
- Derive request/error/latency and service counts from spans; error-log and event counts from their respective tables; machine CPU/RAM from existing infrastructure metrics without loading raw points.
- Cover boundary inclusion (`[from,to)`), nanosecond conversion, absent signals, and host/service isolation with storage query tests or existing database-backed integration helpers.
- Run the affected storage tests, then the timeline summary API tests from Task 1.

### 3. Expose the summary endpoint and generated contract

- Add `GET /api/v1/projects/{project_id}/environments/{environment_id}/timeline/summary?from=&to=&buckets=` with the normal `TelemetryView` authorization.
- Document the route in OpenAPI and regenerate client/schema contracts with `just codegen`.
- Add an integration test that requests a 30-day summary, checks the response is bounded and correctly scoped, and rejects a range beyond 30 days.
- Run: `cargo test -p karbird-api`; targeted integration test.

### 4. Add timeline range loading and stale-result protection

- Add pure range helpers for the 30-day display limit, loading threshold, and adjacent seven-day detail chunks; test left/right edge selection and clamping.
- Track requested and loaded intervals independently. Deduplicate overlapping in-flight chunk requests and merge returned items by stable IDs.
- Keep the current data visible while loading. Ignore results for an old project/environment or superseded request generation.
- Use the summary endpoint for broad views and the existing raw endpoint for detail. Do not add the same range's summary values on top of its raw values.
- Update Worker/client protocols and regenerate contracts where needed; test summary/detail switching, overlapping chunks, and stale responses.
- Run: `pnpm --filter @karbird/web test` for affected files and TypeScript checks.

### 5. Render month overview and loading treatment

- Allow the timeline to zoom out to 30 days while keeping its time axis inside the viewport.
- Draw summary buckets at broad zoom and raw signal detail at closer zoom. Keep vertical row scrolling separate from horizontal time panning.
- Add a left/right edge loading sweep with a restrained luminous beam, gradient trail, and moving signal points. Keep controls interactive; stop on settle; expose a screen-reader status; show a static accent under reduced motion.
- Test loading direction, settling/error states, and reduced-motion rendering.

### 6. Verify as a user

- Add Playwright coverage that signs in, zooms to a month, pans into an unloaded edge, observes the edge loading state, and verifies data appears while previously loaded content remains visible.
- Include reduced-motion and empty/retention-boundary cases without fixed sleeps.
- Run the focused E2E spec, `just check`, `just test`, and required integration/E2E suites against `just dev` and `just demo`.
- Report any environment-blocked checks explicitly. Leave changes in the working tree; do not perform Git operations.

## Review focus

- Integer nanosecond boundaries must stay strings across JSON/TypeScript boundaries.
- Bucket queries must remain tenant scoped and use exclusive `to` boundaries consistently.
- Late live records must not double count against summary buckets.
- Rapid pan/zoom or project switching must not apply stale responses or leave the animation stuck.
- A busy environment must return a bounded summary and must not require fetching a month's raw telemetry.
