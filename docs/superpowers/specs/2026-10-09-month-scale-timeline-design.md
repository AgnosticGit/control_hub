# Month-scale timeline with progressive loading

## Goal

Let users inspect up to 30 days on one timeline, pan through time, and progressively load useful detail without freezing the workspace or flooding the API with requests.

## Current constraints

- Timeline zoom and pan are clamped to the range already loaded into the WASM engine.
- The client loads raw timeline items through the Worker; a timeline request is limited to 50,000 items.
- Hot timeline API reads currently accept at most seven days.
- Timeline tracks are aggregated in the engine from the currently loaded raw items, so asking the existing endpoint for a month would truncate busy environments.

## Design

### Month overview

Add a server-side aggregated timeline read for broad ranges up to 30 days. It returns at most 2,000 time buckets, with request and error counts, p95 latency, per-service span counts, error-log counts, event counts, and per-machine CPU/RAM summaries for the visible tracks. The API applies the authenticated tenant, project and environment scope exactly as existing telemetry routes do. Buckets are summaries; they do not pretend to contain individual spans or log lines. Results only cover data still available under the configured retention/archive policy.

At broad zoom levels, the UI renders these buckets so a month fits across the available timeline width. As the user zooms into a smaller interval, the Worker loads raw timeline details for that interval and renders individual spans, logs and events using the existing engine path.

### Progressive range loading

The visible time range can extend to 30 days even when only part of it is loaded. Panning or zooming near an unloaded edge requests the adjacent time chunk. Raw detail requests remain within the existing seven-day API limit. Requests are deduplicated, scoped to the selected project/environment, and applied to the Worker only if they still match the current workspace and request generation. Loaded records are merged by their stable IDs; overlapping chunks must not duplicate visible signals. The broad summary and raw detail occupy distinct zoom levels so a summary is never added on top of the same raw records.

The currently visible data stays on screen while an adjacent chunk loads. A failed chunk can be retried by moving back into that edge; it must not clear already loaded data or move the playhead.

### Loading feedback

Show a timeline-native loading treatment at the edge being fetched: a narrow luminous sweep with a soft moving gradient and a few restrained signal points traveling in the load direction. Keep the ruler, tracks and controls interactive. Distinguish left and right loads, stop the animation when the request settles, and provide a concise screen-reader status. Respect `prefers-reduced-motion` by replacing the sweep with a static progress accent.

### Interaction and limits

- Keep the existing drag, horizontal-wheel and zoom gestures.
- Allow zooming out to a 30-day visible interval, but do not force a full 30-day raw download.
- Keep vertical scrolling for track rows independent from horizontal time navigation.
- When the user reaches data older than retention or no data exists, show an empty range normally; loading state must not imply that unavailable data is still being fetched.
- Preserve live mode behavior and current short-range responsiveness.

## Verification

- Rust tests cover aggregate bucket boundaries, tenant/project/environment isolation, empty intervals, and the 30-day maximum.
- Web unit tests cover month zoom limits, edge-load window selection, duplicate/overlapping chunk merge, stale request rejection, and loading direction/state.
- Playwright covers zooming out to a month, scrolling into an unloaded edge, seeing the loading sweep, and signals appearing after load; it also verifies that existing content remains visible during loading and reduced-motion feedback is static.
- Run `just check`, `just test`, and the relevant integration/e2e suites against `just dev` and `just demo`.

## Scope

This change adds an aggregate read for overview and uses existing raw timeline data for detail. It does not change telemetry ingestion, retention policy, or the meaning of existing signal types.
