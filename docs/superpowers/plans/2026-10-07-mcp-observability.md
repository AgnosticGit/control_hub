# MCP Observability Expansion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend `karbird mcp` with typed, read-only infrastructure, traffic, timeline, coverage and incident-investigation tools for its token-bound environment.

**Architecture:** Keep MCP as a local stdio adapter over existing Karbird REST endpoints. Centralize MCP-only input validation, time selection and UI-link construction in the CLI, then compose existing typed API calls for the incident snapshot. No API storage route, authorization rule or archive path changes.

**Tech Stack:** Rust, clap, serde_json, ureq, Model Context Protocol over JSON-RPC stdio, Node integration tests.

**Spec:** `docs/superpowers/specs/2026-10-07-mcp-observability-design.md`

## Global Constraints

- Preserve `karbird mcp` as local stdio and `KARBIRD_API_TOKEN` as a read-only, environment-bound token.
- Tool arguments must never choose organisation, tenant, project or environment.
- Return only bounded, recursively sanitized untrusted telemetry; use decimal-string nanosecond times.
- Implement no archive access, write actions, generic REST/SQL tool, Docker or shell control.
- `ui_url` is optional and comes only from `KARBIRD_WEB_URL`; never derive it from the API port and never include a token.
- Do not commit or push. The repository rule reserves both actions for the user.

## Review Focus

- A decimal timestamp above `i64::MAX`, zero, negative or paired with only one boundary must fail before the CLI opens an API request; cover in Task 1.
- An API URL with a path, query or fragment must not create an ambiguous UI URL; cover in Task 2.
- `null` throughput with no declared `Content-Length` must remain `null`, never numeric zero; cover in Task 3.
- A token for another environment must not reveal demo machine names or edge traffic; cover in Task 3.
- A service-filtered incident snapshot must not silently include unrelated errors or non-adjacent topology edges; cover in Task 4.

---

## File structure

- `crates/karbird-cli/src/main.rs` — optional `KARBIRD_WEB_URL` connection setting and CLI parsing tests.
- `crates/karbird-cli/src/mcp.rs` — schemas, time selection, typed route mapping, snapshot composition, response envelopes and unit tests.
- `crates/karbird-cli/src/client.rs` — validation and construction of an optional public web base URL.
- `tests/integration/mcp.test.mjs` — black-box MCP coverage for the expanded tool list and environment isolation.
- `docs/adr/0013-read-only-mcp.md` — extended allowed tool surface and non-goals.
- `docs/integration/AI-INTEGRATION.md` — public AI integration description.
- `docs/integration/CLI.md` — configuration and MCP tool reference.

### Task 1: Add shared MCP time selection

**Files:**
- Modify: `crates/karbird-cli/src/mcp.rs:24-220`
- Test: `crates/karbird-cli/src/mcp.rs:500-730`

**Interfaces:**
- Produces `TimeSelection { from_ns: String, to_ns: String, window_minutes: Option<i64> }`.
- Produces `ToolArgs::time_selection(default_minutes: Option<i64>) -> anyhow::Result<TimeSelection>`.
- Existing range tools consume its `from_ns` and `to_ns`; `get_context` remains minutes-only.

- [ ] **Step 1: Write failing unit tests for valid absolute ranges and invalid selector combinations**

Test `from_ns=1000000000,to_ns=2000000000`, missing one boundary, `minutes` mixed with absolute time, zero, negative, overflow and inverted values. Assert invalid inputs create an MCP tool error before any HTTP request is recorded.

- [ ] **Step 2: Run the MCP unit test target to verify the new tests fail**

Run: `cargo test -p karbird-cli mcp::tests -- --nocapture`

Expected: FAIL because `time_selection` and absolute schemas do not exist.

- [ ] **Step 3: Implement the typed selector and reusable schema fields in `mcp.rs`**

Accept only decimal positive `i64` strings for `from_ns` and `to_ns`; require both together; reject `minutes` when absolute boundaries appear. Emit API query keys `from` and `to`, keep relative default windows for existing range tools, and never convert timestamps through floating point.

- [ ] **Step 4: Run the MCP unit test target to verify it passes**

Run: `cargo test -p karbird-cli mcp::tests -- --nocapture`

Expected: PASS.

### Task 2: Add safe UI links to MCP envelopes

**Files:**
- Modify: `crates/karbird-cli/src/main.rs:25-48, 92-118, 180-220`
- Modify: `crates/karbird-cli/src/client.rs:40-140, 240-300`
- Modify: `crates/karbird-cli/src/mcp.rs:150-280, 500-730`
- Test: `crates/karbird-cli/src/main.rs:180-220`
- Test: `crates/karbird-cli/src/client.rs:240-300`
- Test: `crates/karbird-cli/src/mcp.rs:500-730`

**Interfaces:**
- `Conn` gains `web_url: Option<String>` from `KARBIRD_WEB_URL` / `--web-url`.
- `Client` exposes `ui_url(&self, ui_path: &str) -> Option<String>`.
- `Server::envelope(data, selection, focus) -> Value` adds `ui_path` and optional `ui_url`.

- [ ] **Step 1: Write failing tests for CLI parsing and web URL validation**

Test valid `https://karbird.example`, loopback `http://127.0.0.1:3000`, rejected remote HTTP, credentials, path/query/fragment, and that API URL never becomes the web URL by default.

- [ ] **Step 2: Run the focused CLI client tests to verify they fail**

Run: `cargo test -p karbird-cli client::tests main::tests -- --nocapture`

Expected: FAIL because `web_url` is absent.

- [ ] **Step 3: Implement `KARBIRD_WEB_URL` and deterministic deep-link building**

Build a relative `/app` route with percent-encoded project, environment, time selector and optional focus parameters. Return `ui_path` for every successful tool result. Resolve `ui_url` only against a validated explicit web base URL. Keep all tokens out of link paths and query values.

- [ ] **Step 4: Run focused tests and MCP envelope tests**

Run: `cargo test -p karbird-cli client::tests mcp::tests -- --nocapture`

Expected: PASS.

### Task 3: Expose infrastructure and edge-traffic MCP tools

**Files:**
- Modify: `crates/karbird-cli/src/mcp.rs:30-280`
- Test: `crates/karbird-cli/src/mcp.rs:500-730`
- Test: `tests/integration/mcp.test.mjs:70-180`

**Interfaces:**
- `get_infrastructure({ at_ns? })` calls `infrastructure?at=`.
- `get_infrastructure_timeline({ from_ns, to_ns, buckets? })` calls `infrastructure/timeline`.
- `get_edge_traffic({ source, target, relation, from_ns, to_ns, buckets? })` calls `edges/traffic`.
- All tools return the common MCP envelope from Task 2.

- [ ] **Step 1: Write failing unit tests for route/query generation**

Use `live_server` to assert the exact endpoint and encoded query parameters for all three tools. Assert `buckets` clamps only through the API's documented maximums in the tool schema: 1–1000 for infrastructure, 1–600 for edge traffic. Assert invalid blank/control-character edge IDs and equal endpoints return a tool error before HTTP.

- [ ] **Step 2: Run focused MCP unit tests to verify they fail**

Run: `cargo test -p karbird-cli mcp::tests -- --nocapture`

Expected: FAIL because the new tools are not in `TOOLS` or dispatch.

- [ ] **Step 3: Implement typed schemas and dispatch**

Add only the three named tools. Preserve `avg_*_bps`, `peak_*_bps` and coverage values as returned from the API; do not recompute rates or coerce JSON `null` to zero. Add deep-link focus for the infrastructure moment and selected edge.

- [ ] **Step 4: Add black-box integration coverage for infrastructure, edge traffic and token isolation**

Extend `mcp.test.mjs` to wait for demo infrastructure, call each tool with the existing demo scope, assert a machine/container model and edge-traffic response, then start an MCP client with the isolation token and assert it cannot see demo machine or edge identifiers.

- [ ] **Step 5: Run unit and integration MCP tests**

Run: `cargo test -p karbird-cli mcp::tests -- --nocapture`

Run: `just test-integration -- mcp.test.mjs`

Expected: PASS with `just dev` and `just demo` running.

### Task 4: Expose timeline, coverage and incident snapshot

**Files:**
- Modify: `crates/karbird-cli/src/mcp.rs:30-280`
- Test: `crates/karbird-cli/src/mcp.rs:500-730`
- Test: `tests/integration/mcp.test.mjs:70-180`

**Interfaces:**
- `get_timeline({ from_ns, to_ns, limit? })` calls `timeline` with a bounded item limit.
- `get_coverage({ from_ns, to_ns })` calls `coverage`.
- `get_incident_snapshot({ minutes? | from_ns,to_ns, service_id? })` returns `{ services, topology, traces, logs, events, coverage, infrastructure }` within the common envelope.

- [ ] **Step 1: Write failing unit tests for snapshot composition**

Simulate each endpoint in `live_server`. Assert the snapshot sends only error-trace and error-log filters, scopes optional service filters to traces/logs, removes non-adjacent topology edges for `service_id`, and fails the whole tool when one required request fails.

- [ ] **Step 2: Run focused MCP tests to verify they fail**

Run: `cargo test -p karbird-cli mcp::tests -- --nocapture`

Expected: FAIL because timeline, coverage and snapshot dispatch do not exist.

- [ ] **Step 3: Implement the three typed query tools and composed snapshot**

Use the common absolute selector. Give timeline a schema item limit of 1–1000 (the existing API has no timeline bucket parameter). Bound snapshot nested arrays with existing `context::sanitize`; keep only API facts and deterministic infrastructure hints. Do not fetch traffic for every topology edge.

- [ ] **Step 4: Extend integration tests for exact-time investigations**

Call timeline, coverage and snapshot with a short explicit time range. Assert decimal-string times, `ui_path`, the untrusted-data notice, data completeness and service-filtered results. Assert malformed and inverted times fail.

- [ ] **Step 5: Run focused tests**

Run: `cargo test -p karbird-cli mcp::tests -- --nocapture`

Run: `just test-integration -- mcp.test.mjs`

Expected: PASS with the stack running.

### Task 5: Update the MCP contract documentation and full verification

**Files:**
- Modify: `docs/adr/0013-read-only-mcp.md`
- Modify: `docs/integration/AI-INTEGRATION.md`
- Modify: `docs/integration/CLI.md`
- Test: `tests/integration/mcp.test.mjs`

**Interfaces:**
- Documents the exact public MCP tool names and `KARBIRD_WEB_URL`.
- States that archive search and mutations remain unavailable.

- [ ] **Step 1: Update ADR and integration documentation from the implemented schemas**

Document every new tool, exact time inputs, explicit web base, server-side scoping, `null` rate meaning and non-goals. Do not promise MCP project selection, remote transport or action execution.

- [ ] **Step 2: Update the tool-list integration assertion**

Assert the final full sorted tool list and read-only annotations. Keep the existing bad-argument, revocation and token-secrecy tests.

- [ ] **Step 3: Run static and unit verification**

Run: `just check`

Run: `just test`

Expected: PASS.

- [ ] **Step 4: Run end-to-end verification for API-facing behavior**

Run: `just test-integration`

Run: `just test-e2e`

Expected: PASS with `just dev` and `just demo` running. If either runtime suite cannot run, report the blocker and do not claim it passed.

## Plan self-review

- Spec coverage: Tasks 1–4 implement every included read-only capability; Task 5 documents it. Archive, remote transport and write operations remain explicitly untouched.
- Type consistency: all MCP range tools share `TimeSelection`; all responses use one envelope builder; all deep links use one validated `Client::ui_url` path.
- Review Focus coverage: timestamp validation (Task 1), web URL ambiguity (Task 2), unknown byte rate (Task 3), tenant isolation (Task 3 integration), service-filtered snapshot (Task 4).
- Proportion: the plan specifies public names, bounds and tests without prescribing implementation bodies.
