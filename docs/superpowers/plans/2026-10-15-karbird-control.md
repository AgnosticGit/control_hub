# Karbird Control Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An owner can enroll one Docker container and authorized users can start, stop, and restart it through the existing audited action flow.

**Architecture:** A standalone `karbird-control` binary polls outward to dedicated API endpoints using a scoped credential. The existing executor dispatches approved typed plans through a durable command table and records results. Docker access stays on the customer's host; Collector remains read-only and separate.

**Tech Stack:** Rust, Axum, PostgreSQL/SQLx, reqwest, Docker Engine HTTP over Unix socket, Next.js/React, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-15-karbird-control-design.md`

## Global Constraints

- Git is read-only: no commits, branches, staging, resets, or configuration changes.
- One installation allows exactly one Docker container ID and only start, stop, restart.
- No free-form commands or telemetry-derived targets; no plaintext stored credentials.
- API and web never access Docker; all action state changes are audited.
- `just codegen` after API or engine type changes; `just check`, `just test`, integration, and E2E before completion.

## Review Focus

- A deleted and recreated container with the same name must be rejected until re-confirmed.
- A reused/expired enrollment code or revoked credential must fail without revealing secrets.
- A duplicate command delivery must not repeat a restart.
- Loss of Control after Docker accepted a command must produce an unknown outcome, not a false success or automatic retry.
- A stopped container must be verified by Docker state, not by expected telemetry.

---

### Task 1: Persistence and command invariants

**Files:** Create `infra/migrations/postgres/0017_control.sql`; create `crates/karbird-storage/src/control.rs`; modify `crates/karbird-storage/src/lib.rs`.

**Interfaces:** `ControlInstallation` and `ControlCommand` rows; storage methods for code creation/atomic consumption, confirmation, revocation, credential lookup, heartbeat, command creation/claim/completion. Command ID is unique per plan; claim is atomic and scoped by installation. All secrets are stored as hashes.

- [ ] Write storage tests for scope isolation, expired/reused code, revoked credential, command lease and duplicate completion.
- [ ] Run affected tests and observe failure.
- [ ] Add migration and methods; run the tests until they pass.

### Task 2: Control HTTP protocol

**Files:** Create `crates/karbird-api/src/routes/control.rs`; modify `crates/karbird-api/src/routes/mod.rs`, `main.rs`, `openapi.rs`; regenerate `packages/api-client`.

**Interfaces:** Owner session routes create/list/confirm/revoke installation; dedicated bearer routes enroll, heartbeat, poll, complete. Responses carry command ID, fixed verb, bound container ID and expiry; never carry Docker host addresses or shell text.

- [ ] Write handler tests for success, 401/403, CSRF, tenant isolation and revoked/expired credentials.
- [ ] Observe failures; implement routes and storage calls.
- [ ] Run handler and integration tests, then `just codegen`.

### Task 3: Typed action and executor dispatch

**Files:** Modify `crates/karbird-actions/src/lib.rs`, `crates/karbird-api/src/routes/actions.rs`, `crates/karbird-executor/src/main.rs`; create `crates/karbird-executor/src/control.rs`.

**Interfaces:** `container.start`, `.stop`, `.restart` resolve an installation UUID to a confirmed container ID. All three share the one-open-plan key. Executor creates one durable command per plan, waits while renewing the plan lease, and records attested Docker state or unknown outcome; stop skips telemetry verification.

- [ ] Add failing unit/property and handler tests for dynamic targets, unconfirmed/revoked targets, policy, and repeated claims.
- [ ] Implement catalog, validation, dispatch and result mapping.
- [ ] Run crate tests and a real API→executor→fake Control integration test.

### Task 4: Local Karbird Control binary

**Files:** Create `crates/karbird-control/Cargo.toml`, `src/lib.rs`, `src/main.rs`, Docker client and durable journal modules; add versioned Dockerfile and Compose example under `deploy/control/`.

**Interfaces:** Configuration contains HTTPS Karbird origin, secret file, state directory and exact Docker container ID. Enrollment exchanges a one-time code for a scoped credential. Poll loop rejects mismatched/expired commands, writes command ID to journal before local Docker call, inspects before/after, and returns saved outcome on duplicate delivery.

- [ ] Add failing tests with a fake Unix Docker socket for each verb, ID mismatch, health timeout and duplicate command ID.
- [ ] Implement the binary without shell invocation or a remote Docker TCP endpoint.
- [ ] Run crate tests and a disposable-container integration test.

### Task 5: Setup and action UI

**Files:** Add setup components/routes under `apps/web/src/app/app` and `apps/web/src/components/actions`; update action catalog client types and existing map controls.

**Interfaces:** Owner can issue/revoke code, view reported container and confirm its immutable ID; authorized roles see only applicable verbs, exact target, reason, status and audit link. Offline/unconfirmed installations show an explanation.

- [ ] Add nearby Vitest tests and Playwright specs for enrollment, confirmation, permissions, approval and all verbs.
- [ ] Observe failures; implement UI and client calls.
- [ ] Run targeted web and E2E tests.

### Task 6: Operational documentation and full verification

**Files:** Update `docs/adr/` with the Control decision, repository map and deployment documentation; add install/upgrade/revoke guidance.

- [ ] Document Docker socket privilege and secret-file handling accurately.
- [ ] Run `just codegen`, `just check`, `just test`, `just test-integration`, `just test-e2e`, and the separate broker-outage test if the data path changes.
- [ ] Inspect `git diff --check` and `git status --short`; report all edited files and unrun checks.
