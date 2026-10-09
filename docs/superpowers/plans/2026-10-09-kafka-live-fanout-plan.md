# Kafka live fan-out implementation plan

> **For agentic workers:** Execute natively in the current user workspace. Never create branches or commits; the developer owns all Git operations.

**Goal:** Migrate the workspace from `rskafka` to `rdkafka` and replace per-WebSocket ClickHouse polling with Kafka-backed live fan-out while preserving processor durability and HTTP recovery.

**Architecture:** `karbird-ingest` uses a keyed `FutureProducer`; `karbird-processor` uses a manually committed consumer group while retaining PostgreSQL as its authoritative processed watermark; each API instance uses a distinct `rdkafka` live consumer group and a bounded scope-keyed local hub. The processor converter becomes shared pure code. WebSockets subscribe to bounded queues and use ClickHouse only for one-time catch-up/history/recovery.

**Tech stack:** Rust, Tokio, `rdkafka`/librdkafka, PostgreSQL offsets, ClickHouse, axum WebSocket, existing TypeScript/WASM timeline protocol.

**Spec:** `docs/superpowers/specs/2026-10-09-kafka-live-fanout-design.md`

## Global constraints

- No Git commits, pushes, branch creation, resets, or history changes.
- Preserve ingest ACK only after Kafka broker acknowledgement.
- Preserve processor at-least-once semantics: ClickHouse + PostgreSQL completion before Kafka offset commit.
- Disable automatic consumer offset commits and automatic offset store for both processor and live consumer.
- A live consumer group is unique per API replica so each local hub sees all partitions.
- No periodic ClickHouse queries from the WebSocket loop; no unbounded queues.
- Preserve tenant/project/environment isolation, origin validation, session checks, existing WebSocket wire shapes, and HTTP gap recovery.
- Add regression/unit/integration/e2e tests according to repository testing policy.

## Review focus

- Existing PG offsets are honored on first startup after migration; no silent skip or full retained-topic replay.
- Producer key partitioning preserves per-environment ordering; delivery failure is an ingest error, never success.
- Slow WebSocket, overflow, API restart, rebalance, broker outage, and duplicate replay have explicit safe behavior.
- Malformed envelopes do not panic and cannot leak across scopes.
- Windows, Linux CI, and production Docker builds have the native toolchain required by librdkafka.

---

### Task 1: Migrate shared broker runtime and ingest producer to rdkafka

**Files:**
- Modify: `Cargo.toml`, `crates/karbird-runtime/Cargo.toml`, `crates/karbird-runtime/src/lib.rs`
- Modify: `crates/karbird-ingest/Cargo.toml`, `crates/karbird-ingest/src/main.rs`, `crates/karbird-ingest/src/broker.rs`
- Test: producer unit tests and runtime connectivity/configuration tests

**Interfaces:**
- Shared runtime creates configured `rdkafka` clients with bounded bootstrap timeout/retry.
- Ingest publishes keyed records through `FutureProducer`, waits for delivery acknowledgement, and maps timeout/broker/overload errors to existing retryable ingest errors.
- Topic metadata determines current partition count; the existing stable environment-key hash selects the partition explicitly to preserve historical partition placement.

- [x] Add/retain producer tests for stable key routing assumptions and delivery ACK/error mapping.
- [x] Replace the custom Kafka producer with `FutureProducer`, retaining bounded memory/queue settings and broker acknowledgements.
- [x] Remove all runtime `rskafka` dependencies, logs, and APIs.
- [x] Run focused ingest/runtime tests.

### Task 2: Migrate processor to consumer groups with PG offset continuity

**Files:**
- Modify: `crates/karbird-processor/Cargo.toml`, `crates/karbird-processor/src/main.rs`, storage offset methods if needed
- Test: processor offset/rebalance unit tests and integration coverage

**Interfaces:**
- [x] Processor joins stable group `karbird-processor`; automatic commits are disabled.
- [x] On each delivered partition batch, the processor reloads PostgreSQL's offset and seeks before processing if Kafka's group position differs. If absent, it starts from earliest.
- [x] For each completed batch, ClickHouse inserts and dead-letter writes finish, then PostgreSQL offset/watermark commits, then Kafka group offset is enqueued asynchronously.
- [x] Reassignment reloads PG offset for every batch; a race during seek is retried on the next delivery.

- [x] Add tests for offset selection, including existing PG offset ahead of Kafka and missing PG offset.
- [x] Use consumer-group partition assignment and ensure no received batch is processed before PG offset reconciliation.
- [x] Convert processor loops from direct partition fetch to consumer groups and bounded batches; preserve lag/watermark metrics and retry behavior.
- [x] Run focused processor tests. Broker-backed integration is covered by the workspace integration suite when dev/demo is available.

### Task 3: Extract canonical converter and add bounded live hub

**Files:**
- Create: `crates/karbird-convert/Cargo.toml`, extracted pure conversion code
- Modify: workspace members/dependencies, processor and API manifests/modules, API state
- Create: `crates/karbird-api/src/live_hub.rs`
- Test: converter fixtures and hub unit tests

**Interfaces:**
- Shared converter yields canonical scope plus normalized rows/items for spans, logs, events, and metrics without storage I/O.
- Hub publishes only to exact authorized scope keys; subscriber queues are bounded and overflow is explicit.

- [x] Pin existing converter behavior with fixtures for trace, log, event, and malformed messages.
- [x] Add hub tests for scope isolation, replay, overflow, and dropped subscriber cleanup.
- [x] Extract converter while keeping metric normalization in processor output unchanged.
- [x] Make API timeline mapping consume shared normalized rows and verify identical item IDs/content.
- [x] Run focused converter/API tests.

### Task 4: Add API live consumer group and WebSocket fan-out

**Files:**
- Create: API live consumer module
- Modify: `crates/karbird-api/src/main.rs`, `state.rs`, `routes/live.rs`, API manifest
- Test: consumer offset tests, WebSocket protocol/auth tests, integration test

**Interfaces:**
- API has a unique configurable live group ID per replica; one default suffices for the current single API deployment.
- Live consumer decodes messages, publishes scoped timeline items, disables auto commit, and commits each offset only after hub admission.
- WebSocket subscribes to the local scope queue and retains compatible `hello`, `batch`, `gap`, `heartbeat`, and `error` frames.
- Overflow emits `gap` and triggers existing client HTTP recovery; slow sockets do not stall consumer or other subscribers.

- [x] Add tests for group configuration and scope fan-out; commit-after-admission is enforced by the consumer flow and broker-backed live delivery is verified by integration tests.
- [x] Start the consumer with API serving; fail startup if required Kafka consumer cannot initialize.
- [x] Replace the poll/ClickHouse socket loop with one catch-up, queue receive, heartbeat, and session recheck.
- [x] Expose aggregate lag, decode and offset-commit enqueue errors, hub backpressure, subscriber overflows, and scope evictions through a private Prometheus `/metrics` endpoint with no tenant labels.
- [x] Run focused API and web-engine protocol tests.

### Task 5: Update build/deploy docs and full verification

**Files:**
- Modify: production Docker build stage, local doctor/tool requirements, CI workflow as needed, Kafka ADRs, operations docs
- Test: integration and Playwright e2e suites

- [x] Add CMake/C++ build prerequisites for local Windows/Linux builds and production image builds; runtime images use the statically built client.
- [x] Update ADR-0002 and ADR-0003 to document rdkafka, group IDs, manual commits, and PG offset migration.
- [x] Add integration coverage for processor continuity and live delivery/recovery; e2e retains the live UI and gap recovery paths.
- [x] Run `just check`, full Rust workspace tests, SDK/frontend/demo/script tests (PHP through a disposable Composer container), and the full integration suite. The full Playwright run had 121 passed and 31 failures against the stale pre-change local processes; the updated API's live WebSocket browser test passed separately.
- [x] Review modified source/config/docs and ensure no stale runtime `rskafka` usage remains (historical ADR title/reference intentionally retained).
- [ ] Run `graphify update .`; leave changes uncommitted.

## Execution choice

The user explicitly requested full implementation in the current workspace and no commits/branches. Execute natively; perform one final independent review only if a suitable review tool is available without creating Git state.
