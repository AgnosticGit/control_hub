# Timeline WebSocket loading and browser cache

## Goal

Load historical timeline ranges through the already-open, authenticated WebSocket, persist every received timeline signal in the browser, and animate newly appearing history without repeatedly downloading completed ranges.

## Current behavior

- The engine Worker owns the live WebSocket and WASM engine.
- Live spans, logs, and events arrive over WebSocket; historical timeline ranges are fetched over HTTP by the Worker.
- `loadCompleteTimeline` recursively splits truncated ranges into smaller HTTP requests.
- Timeline rows are drawn to a canvas from engine aggregates. React does not render each signal as an individual DOM element.

## Design

### Historical range protocol

The Worker sends one scoped `timeline.load` message containing `request_id`, the requested range, uncovered half-open subranges, and the bounded per-signal item limit. The API derives tenant, project, environment, and permissions from the authenticated WebSocket session; scope is never accepted from the message.

The API queries requested subranges, recursively subdividing capped intervals on the server as needed. It sends `timeline.chunk` frames in sequence, each containing a request ID, sequence number, interval, and timeline items. Only one frame may be outstanding; the client acknowledges it after the Worker has applied and persisted it. `timeline.range_complete` carries the next sequence number and is also acknowledged only after coverage is committed. The final `timeline.complete` frame reports overall completeness and truncation. Invalid ranges, permission failures, server limits, and query failures return a request-scoped error without terminating unrelated live delivery.

The Worker multiplexes history frames with existing live signals, heartbeats, and edge updates. A new range can cancel an obsolete request. If the socket closes, the Worker retries only still-uncovered subranges after reconnect; fully acknowledged intervals are omitted, while an interrupted leaf interval may replay from its beginning. Already-applied items are deduplicated by stable timeline identity. It never marks a partial interval complete or treats it as a cache hit. If bounded server splitting still cannot return a full range, the UI preserves an explicit incomplete/truncated state.

### IndexedDB cache

The Worker stores all history items received from timeline chunks and all live timeline signals in IndexedDB, partitioned by authenticated account/scope and time range. Timeline items include spans, logs, and events in the existing safe timeline DTO shape; this cache is for timeline rendering and does not claim to contain separately fetched full trace trees or full log detail responses.

Each leaf interval that receives `timeline.range_complete` without truncation is recorded as complete coverage. When the user revisits a fully cached interval, the Worker applies it locally and does not request those records again. A request covering a mix of cached and missing time loads only the uncovered intervals. Persist each completed interval transactionally so an interrupted write cannot create false coverage. Stable item identities deduplicate overlaps between cached ranges and live signals.

The cache has a hard target cap of **1 GiB** measured from serialized UTF-8 payload sizes. On reaching the cap, evict the oldest completed time ranges first, preserving newer ranges. If the browser's quota is lower, estimate available storage, evict old ranges and retry once; if IndexedDB still rejects the write, keep the live/timeline session usable without caching. Cache failures must never block applying data to the WASM engine.

Use an explicit IndexedDB version for the timeline cache. On a version mismatch, clear all timeline records and coverage metadata before accepting new entries. Partitioning includes account and scope; logout clears that account's cache. A browser storage `versionchange` closes open connections cleanly.

### Timeline appearance

When a historical interval is applied, the canvas briefly animates only the newly loaded time region with a restrained opacity/glow reveal. Existing signals, cursor, and ruler remain stable; repeated seeks and ordinary canvas redraws do not replay the effect. Live signals use the same visual treatment in bounded short windows. `prefers-reduced-motion` switches to a static accent, and animation state is cleared after completion or cancellation.

### Reliability and resource bounds

- WebSocket ordering plus one outstanding acknowledged chunk bounds server and browser buffers on slow links.
- An incomplete transfer retries only uncovered intervals after reconnect; no partial data is advertised as complete.
- Retry is idempotent: the Worker may receive earlier chunks again and applies stable-ID deduplication.
- Server-side limits cap maximum requested interval, chunk size, outstanding work per socket, and total split/query budget.
- While offline, cached ranges remain available; missing ranges remain visibly pending and retry when the socket reconnects. Delivery cannot complete until connectivity returns.
- WebSocket history does not replace live fan-out or move database work to the browser; ClickHouse remains the historical source of truth.

## Tests

- API tests validate authenticated scoping, interval bounds, ordered chunk sequence, ACK backpressure, cancellation, completion/truncation, and malformed control frames.
- Worker tests validate one initial request per range, multiplexing with live events, retry of only uncovered ranges, chunk ACK only after apply/persist, deduplication, cancellation, and partial ranges never being cached as complete.
- IndexedDB tests validate full cache invalidation on version change, 1 GiB accounting/oldest-first eviction, uncovered-range selection, quota failure fallback, and scope isolation.
- Canvas/component tests validate reveal range isolation and reduced-motion behavior; Playwright verifies signals visibly appear after a range load without replacing existing timeline content.
- Run `just check`, `just test`, `just test-integration`, and `just test-e2e` with the local stack and demo running.

## Non-goals

- Caching unrelated API pages or full trace/log inspector payloads.
- Claiming delivery while the browser is offline.
- Changing telemetry retention, timeline item semantics, or the ClickHouse schema.
