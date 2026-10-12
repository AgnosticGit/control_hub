# Benchmark Coverage and Measured Optimization Plan

> **Execution:** Native in the current workspace. Git operations are prohibited by `AGENTS.md`.

**Goal:** Extend meaningful Criterion coverage for CPU-heavy production paths and optimize only measured bottlenecks without changing business behavior or contracts.

**Architecture:** Keep benchmarks beside their owning Rust crates and call production APIs. Record a same-machine Criterion baseline before each optimization, retain or add behavioral tests for the affected invariant, then compare against the saved baseline. Do not benchmark external I/O, trivial accessors, or synthetic code that does not represent production work.

**Tech Stack:** Rust workspace, Criterion 0.8, existing unit/property suites.

## Global Constraints

- Preserve API, output ordering, validation, redaction, and persisted/wire formats.
- Benchmark deterministic fixtures; exclude fixture construction from timed work.
- Accept an optimization only when the same-machine Criterion comparison shows a repeatable improvement without a meaningful regression elsewhere.
- No commits, pushes, branches, worktrees, or other Git operations.

## Review Focus

- Layout and topology rebuild costs must represent realistic graph shapes and preserve deterministic geometry.
- Sanitization optimizations must keep sensitive values removed/redacted, including mixed-case and nested attributes.
- Infrastructure grouping/placement optimizations must preserve deterministic ordering and ambiguity handling.
- Archive indexing must preserve bloom-filter no-false-negative behavior and sorted template IDs.
- Benchmarks must time production code, not setup or I/O.

---

### Task 1: Establish current benchmark baseline

**Files:** existing Criterion targets across `crates/*/benches/`.

- [x] Inventory all Rust packages and existing benchmark targets.
- [x] Run the current workspace benchmark suite and record representative intervals and any existing Criterion regressions.
- [x] Inspect the measured CPU paths and select only high-value uncovered workloads and optimization candidates.

### Task 2: Fill material benchmark gaps

**Files:**
- Modify `crates/karbird-engine-core/benches/engine.rs` for topology/layout rebuilds and JSON input processing if those represent distinct hot paths.
- Modify `crates/karbird-api/benches/infrastructure.rs` for resource placement if its pure transformation is material.
- Modify `crates/karbird-ingest/benches/sanitize.rs` for log sanitization and any uncovered repeated validation path.
- Modify `crates/karbird-convert/benches/normalize.rs` for span/log conversion where separate CPU work is not already represented.
- Modify `crates/karbird-archive/benches/segments.rs` for index/template paths if profiling shows meaningful CPU cost.

- [x] Add only representative workloads that call production functions and keep setup out of timed iterations.
- [x] Run affected existing correctness tests and compile/run the new Criterion cases.
- [x] Save the current measurement as the before-optimization baseline for any chosen target.

### Task 3: Optimize measured hot paths

**Files:** selected production module(s) and colocated tests, based on Task 1/2 measurements.

- [x] Write or extend behavior tests for the exact invariant before changing production code.
- [x] Run those tests before the optimization and confirm their expected behavior.
- [x] Make the smallest implementation change supported by the profile/benchmark evidence.
- [x] Re-run behavior tests and Criterion against the saved baseline; revert the resource-placement allocation optimization when repeated measurements showed no reliable improvement.

### Task 4: Verify and report

- [x] Run formatter, clippy/checks, and affected crate tests/benchmarks.
- [x] Run `graphify update .`.
- [x] Record changed paths, before/after measurements, confidence/noise caveats, and any unrun checks.

## Results

The workspace already had a Criterion target in every Rust package. Added representative gaps for API resource placement and CPU-utilization fallback, ingest log sanitization, conversion of 1,000 spans and logs, engine JSON batch application and server geometry, archive segment indexing, and in-place domain sanitization.

Measured CPU changes (Criterion, same local machine; results are not portable across hosts):

- Common attribute sanitization: about 59.8 ns before, 34.6 ns after (about 42% faster).
- URL attribute redaction: Criterion reported about 41% faster.
- Ingest sanitization of 1,000 spans with attributes: about 24% faster.
- Ingest sanitization of 1,000 metric points with attributes: about 16% faster.
- Grouping 500 hosts with realistic metrics: about 1.63 ms and about 14% faster against the prior full-suite baseline.
- 500-host utilization-gauge fallback: about 228 µs and about 13% faster against its saved baseline.
- Candidate removal of temporary placement vectors was reverted after the repeat baseline comparison showed no reliable improvement.

Verification: `just check` passed (ESLint reports three existing warnings in `SignalsPanel.tsx`). `just test` passed Rust workspace and Node SDK tests but could not finish the PHP SDK step because `composer` is unavailable. Integration tests passed 179/187; remaining failures were in local-stack WebSocket/edge-traffic/MCP behavior and migration 9 drift. E2E was stopped after 47/163 due long timeouts on the already-running local app.
