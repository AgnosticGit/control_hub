# WebSocket-only Live Updates and Idempotent Traffic Aggregates

## Goal

Deliver live workspace data over the existing authenticated, environment-scoped WebSocket without an HTTP polling fallback. Recover missed live data over that same WebSocket, stream selected-edge traffic changes as soon as each span is processed, and push action-plan changes to connected workspace clients. New live updates are sent individually; the server does not wait to collect a batch.

## Approved scope

1. Remove periodic HTTP timeline polling when WebSocket reconnect attempts fail. Keep bounded reconnect with backoff and expose a clear stale/disconnected state in the UI.
2. Move live gap recovery to the WebSocket protocol. Initial workspace snapshots, historical ranges, explicit searches, pagination and trace details remain HTTP.
3. Keep the HTTP edge-traffic endpoint for the initial graph and historical views. In live mode, subscribe to the currently selected edge and send changed buckets through the existing WebSocket.
4. Persist per-span traffic contributions through the processor/storage path. A repeated Kafka record must not increment a traffic bucket twice. Send each resulting live change immediately, without collecting it into a timed batch.
5. Replace workspace action-list polling with WebSocket updates. Use PostgreSQL notifications so action changes made by any API or executor process reach connected API instances. HTTP remains available for initial action state and explicit history/details.

## Architecture

### WebSocket lifecycle and recovery

- The browser owns one live WebSocket per active project/environment workspace. The Worker remains the sole owner of the socket and large telemetry parsing.
- On connection failure, the client retries with bounded exponential backoff and keeps the UI marked stale. It does not start periodic HTTP requests.
- The server uses the resume cursor and durable ClickHouse rows to recover spans, logs and events. Recovery records are sent as individual WebSocket messages, not grouped into arrays.
- A recovery that exceeds the current catch-up limit is continued with a cursor/page exchange on the same socket. The server must not report a terminal gap that requires the browser to fetch `/timeline` over HTTP.
- Stable record IDs let the client safely ignore duplicates after reconnect. Resume cursors remain decimal nanosecond strings; they are not Kafka offsets.
- HTTP remains the explicit query interface for initial snapshots, historical ranges, filtered/paginated searches and detail views. It is not used as a silent live fallback.

### Immediate live telemetry

- Keep Kafka consumption and storage writes independent from browser sockets.
- Publish each newly consumed live telemetry record to the scope hub and write each WebSocket record as soon as it is ready. Do not aggregate newly arriving spans/logs/events into time-based batches.
- Keep per-socket queues bounded. A slow client is marked stale and resumes from its cursor; it cannot block Kafka consumption or other sockets.
- Heartbeats, authorization checks, origin validation, session rechecks, scope isolation and reconnect limits remain in force.

### Durable edge traffic contributions

- For every eligible client span, the processor derives the same edge identity and byte-size contribution that the current edge-traffic query uses: scope, source node, target node, relation, event-time bucket, stable span ID, request bytes and response bytes when known.
- Persist a narrow contribution row keyed by scope and stable span ID using an idempotent storage key. Kafka redelivery replaces/reuses the same contribution instead of adding a second count.
- Historical and initial edge-traffic queries aggregate the durable contribution rows, preserving existing API response semantics and null/unknown byte-rate behavior.
- After persistence, the processor publishes a traffic-change record with a stable change ID, edge identity, bucket start and contribution values. Redelivery may publish the same change again; clients deduplicate by stable change ID.
- The API's live consumer forwards a change only to sockets subscribed to that edge and scope. A client applies the contribution once and redraws the affected bucket. Selecting another edge replaces the subscription; closing the inspector removes it.
- On reconnect, the browser re-subscribes. The server replays durable contributions after the acknowledged cursor over WebSocket, then continues live delivery. If the selected range is historical, the browser continues using the HTTP aggregate and does not subscribe to live deltas.
- Live values are additive deltas, not repeated full-window recomputations. The initial HTTP snapshot establishes the baseline; stable change IDs prevent duplicates from inflating it.

### Action-plan updates

- Workspace action bars load their initial list from the existing scoped HTTP endpoint.
- PostgreSQL emits transactional notifications when an action plan is inserted or its state/timestamps/verification result changes. Every API replica listens and forwards the scoped change through the existing live hub.
- The WebSocket sends one action-change message per committed change, without timed batching. The client upserts the corresponding action bar by plan ID and invalidates any active detail view.
- After reconnect, the server sends the current scoped action state over the WebSocket before live notifications, so the workspace does not depend on periodic polling.
- Mutations and action details remain HTTP. Notification payloads carry identifiers/revision only; the API reads the authorized scoped DTO before sending it, avoiding trust in arbitrary notification payloads.

## Protocol shape

The live protocol adds typed single-record messages for `span`, `log`, `event`, `edge_traffic`, and `action`. It also adds client control messages for `subscribe_edge`, `unsubscribe_edge`, `resume`, and paginated recovery continuation. Existing `hello`, `heartbeat`, `error`, `infrastructure`, and `topology` messages remain. Unknown messages are ignored safely. Every server-side query uses the authenticated scope; the client cannot choose tenant/project/environment through message payloads.

## Reliability and limits

- Kafka and processor delivery remains at least once. Stable contribution keys and change IDs make writes and client application idempotent.
- ClickHouse remains the durable source for telemetry and edge contribution history. PostgreSQL notifications are a wake-up/fan-out signal, not the durable action source; reconnect sends a current action snapshot.
- Each socket and recovery stream stays bounded. Recovery pages are ordered deterministically and carry an explicit continuation cursor. Queue overflow pauses that socket and resumes through WebSocket recovery.
- If the server is unreachable, the UI remains stale and retries the WebSocket. No HTTP fallback can provide data while the API itself is unavailable.
- High-rate spans result in high-rate individual WebSocket writes, as explicitly requested. The implementation must measure fan-out CPU, bytes, queue depth and dropped/resumed records. No timed coalescing is allowed for the new live edge changes.
- Existing REST contracts remain backward compatible. No ingestion token, tenant-scope, or redaction behavior changes.

## Acceptance criteria

1. Healthy WebSocket live mode makes no periodic HTTP timeline/log/trace/event requests.
2. Repeated WebSocket failures never start HTTP polling; the UI exposes stale state and retries with backoff.
3. Reconnect and overflow recovery deliver all retained records through WebSocket pages, with no missing or duplicate visible records.
4. Each new telemetry signal is emitted individually without a server-side collection interval.
5. Replaying one Kafka client span leaves its durable traffic contribution unchanged; a new eligible span changes exactly one edge bucket by one call and any known byte sizes.
6. A live edge subscriber receives only changes for its selected edge and scope; non-subscribers receive none.
7. A committed action transition reaches connected clients across API instances; reconnect restores current action state without polling.
8. Initial snapshots, historical queries, log pagination, searches and trace details still use the existing HTTP APIs.
9. API and Worker tests cover resume, page continuation, duplicate delivery, overflow, authorization/scope isolation, edge contribution idempotency, edge subscriptions, PostgreSQL notifications and action reconnect snapshots.

## Constraints

- Follow `AGENTS.md`: no Git commands/operations, no commits or pushes, no hand edits to lockfiles, no changes to business/API behavior beyond this approved scope, and run required checks/tests where dependencies are available.
- No timed batching for new live signals, edge changes or action changes.
- Do not expose raw telemetry or notification payloads as HTML or instructions.
- Generated API contracts must be regenerated if any REST DTO or route changes; the WebSocket protocol is separately typed and tested.
