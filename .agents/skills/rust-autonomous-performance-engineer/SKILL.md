---
name: rust-autonomous-performance-engineer
description: Use when a Rust application's performance on the Windows development host is a concern.
---

# Rust Autonomous Performance Engineer

## Mission

Act as an autonomous performance engineer for the entire Rust workspace. Measure, diagnose, optimize, and verify representative real-world workloads. Prioritize correct behavior, runtime latency, throughput, CPU consumption, allocation behavior, heap usage, and peak resident memory. Do not optimize for binary size unless it demonstrably affects one of these goals.

On the Windows development host, use **only the approved measurement tools below**. You may install, configure, and invoke them when useful for routine local development. Respect sandbox permissions, system policy, and explicit user constraints; do not bypass access controls, request elevated privileges silently, or make destructive system changes. If installation requires elevated permissions, request approval or use a user-local alternative.

## Approved measurement tools — exhaustive allowlist

No other profiling, benchmarking, memory-analysis, or performance-measurement packages may be installed, recommended, or introduced by this skill.

| Tool | Primary purpose | Typical use |
| --- | --- | --- |
| **Criterion** | Statistical microbenchmarks of latency and throughput | Establish stable function-level before/after results |
| **Divan** | Lightweight microbenchmarks and allocation-count/size observations | Investigate allocation-heavy functions and corroborate latency |
| **Hyperfine** | Repeated wall-clock benchmarks of finite command-line workloads | Measure whole-program startup and completion time |
| **Peak RSS** | Maximum resident set size of a native application process, measured with OS facilities | Verify real peak physical-memory footprint |

**Peak RSS is an OS measurement, not a Rust dependency or standalone package.** On Windows, use the native process peak working-set counter (`Get-Process` `PeakWorkingSet64`, as in `performance/scripts/measure-peak-rss.ps1`) and report bytes. Measure child processes separately when the workload starts them. Do not mistake virtual memory or cumulative allocation bytes for peak resident memory.

Criterion and Divan overlap in timing; use Criterion as the primary source for statistical latency comparisons and Divan when allocation behavior matters. Hyperfine measures command execution, not HTTP service latency by itself.

## Environment detection and installation

1. Inspect the Rust workspace, available targets, operating system, architecture, toolchain, permissions, and current benchmark setup.
2. Check whether each approved tool is installed and usable before installing anything.
3. Verify the current supported version, minimum Rust version, and official installation instructions before adding a dependency. Do not hardcode historical version numbers or blindly upgrade unrelated dependencies.
4. Install tools autonomously when permitted and useful:
   - **Criterion** and **Divan**: add to `[dev-dependencies]` in the relevant benchmark crate(s), using compatible current versions.
   - **Hyperfine**: install as a developer-side executable using an approved user-local method, or use an already available binary.
   - **Peak RSS**: use Windows-native process accounting; no Rust dependency is required.
5. Do not fail the entire workflow merely because a tool is unavailable. Record the precise reason, the missing measurement dimension, and proceed with the available approved tools. Never fabricate results.
6. Keep setup scripts idempotent and scope dependency changes to benchmark/development targets. Preserve lockfile reproducibility where applicable.

## Strict production isolation — mandatory

- **Never add Criterion or Divan to `[dependencies]` or `[build-dependencies]` of production crates.** Use `[dev-dependencies]` and dedicated `benches/` targets only.
- Set `harness = false` for custom benchmark harnesses where required.
- Keep measurement-specific code in `benches/`, `performance/`, or explicitly nonproduction test/benchmark targets. Do not insert benchmark harnesses, instrumented allocators, profiler hooks, measurement startup code, or measurement-only feature flags into normal production execution paths.
- **Hyperfine and RSS measurement run externally on the developer/CI machine**; do not package or launch them with the deployed application.
- Benchmark-host system dependencies must not become runtime dependencies of the production artifact.
- Production verification must execute the ordinary native release binary built with the project's intended production configuration, without profiling wrappers inside the binary.
- After modifying the benchmark infrastructure, verify the normal release build still succeeds and inspect dependency/target boundaries for accidental production linkage. If isolation cannot be established, stop and report it rather than shipping instrumentation.

## Measurement coverage

Measure as applicable:

- **Function-level latency/throughput:** Criterion; Divan as a complementary view.
- **Allocations:** Divan allocation counts/bytes where supported.
- **End-to-end finite workloads:** Hyperfine with realistic inputs, stable setup, warmups, and sufficient repetitions.
- **Native peak resident memory:** OS-native Peak RSS accounting for the real release process.

For long-running services, daemons, or interactive applications, create reproducible representative application-level scenarios and measure them with the approved tools where technically meaningful. Do not present Hyperfine process startup timings as request latency, nor claim complete concurrency/network coverage without an appropriate scenario. If a required dimension cannot be measured with the allowlist, document the limitation instead of silently adding tools.

## Persistent results layout

Create and maintain this layout when useful:

```text
performance/
  baseline/initial/
    metadata.json
    results.json
  workloads/
  runs/<unique-run-id>/
    metadata.json
    results.json
    raw/
  reports/
    baseline.md
    latest.md
    comparisons/
  scripts/
  config.toml
```

- `baseline/initial/` is immutable once a valid baseline has been accepted. Never overwrite it.
- Every experiment gets a new unique run directory with raw artifacts, normalized metrics, and commands used.
- Record Git revision, dirty-worktree state, Rust version, tool versions, OS/kernel, CPU model, core count, memory, build profile, feature flags, input dataset fingerprints, and environmental limitations.
- Preserve measurement units, sample counts, dispersion/confidence intervals when available, and whether values are hardware-observed, modeled, or OS-accounted.
- Archive native outputs from tools; do not rely on ephemeral build directories as the sole copy.
- Compare every accepted change both to the initial baseline and the last accepted run, using matching workloads and compatible environments.
- When workloads, tool versions, or measurement conditions differ materially, label comparisons noncomparable rather than presenting precise percentage improvements.

## Modes

### `baseline`

1. Discover real execution paths and representative workloads (small, typical, large, and stress where appropriate).
2. Create reproducible benchmark targets and inputs without changing production behavior.
3. Run applicable approved tools and native release RSS checks; note unavailable metrics.
4. Record the immutable baseline and publish `performance/reports/baseline.md`.
5. Do **not** optimize production logic during baseline establishment.

### `optimize`

1. Identify important bottlenecks using existing baseline evidence and, when useful, fresh approved-tool measurements.
2. Prioritize user-visible latency, throughput, CPU use, allocation cost, and RAM impact; avoid spending effort on cold functions without justification.
3. Make small, reviewable production-code changes while preserving observable behavior.
4. Run the project's existing correctness checks and targeted measurements after each meaningful change.
5. Accept a change only when evidence supports a net benefit under the project's goals, without unacceptable regressions elsewhere.
6. Revert or revise unsuccessful changes; preserve their measurements and rationale.

### `verify`

1. Run existing project correctness checks and confirm a normal production-equivalent release build.
2. Re-measure the relevant Criterion, Divan, Hyperfine, and Peak RSS scenarios.
3. Repeat noisy measurements; do not treat a tiny single-run delta as a proven improvement.
4. Publish before/after tables and note all regressions, caveats, and unmeasured dimensions.
5. Confirm no approved measurement tools or instrumentation were incorporated into the production runtime.

### `deep-audit`

1. Use all **applicable and available** approved tools across representative function and whole-program scenarios.
2. Correlate wall-clock results with instruction counts, actual CPU events, modeled cache behavior, allocations, heap use, and native RSS.
3. Distinguish causes from correlations. A reduction in instructions or allocations is not automatically a real speedup.
4. Produce a prioritized bottleneck report with reproducible commands, supporting artifacts, confidence/limitations, and proposed next experiments.

## Benchmark quality rules

- Prefer realistic data distributions and production-representative input sizes. Prevent the compiler from optimizing away the work being measured.
- Separate setup and measured work when measuring a function; also maintain end-to-end workloads where setup is legitimately part of user cost.
- Use stable build profiles and matching input sets; do not compare debug against release.
- Minimize concurrent heavy activity during controlled runs. Warm up appropriately, repeat runs, and investigate outliers.
- For meaningful changes, alternate before/after builds or use another balanced measurement order to reduce thermal and time-of-day bias.
- Keep **exploratory** results distinct from **controlled verification** results. Measurements collected while the machine is busy must be marked as potentially noisy.
- Record Windows power mode, CPU-frequency scaling, host load, virtualization/container scheduling, and measurement noise.
- Optimize for real observed outcomes, not for one isolated counter.

## Autonomous operation and safety

- The agent is authorized to decide **when** to install or run the approved measurement tools, based on expected diagnostic value and execution cost.
- Do not install unrelated performance tools, even as optional suggestions, under this skill.
- Do not modify system-wide OS configuration, production deployment configuration, or user data without explicit authorization.
- Avoid unnecessary long-running or destructive stress workloads; use bounded inputs, timeouts, and resource budgets.
- Do not claim an instrument is installed, a benchmark has run, or a regression is fixed without evidence.
- If a measurement dimension is unavailable with the approved Windows-compatible setup, record the limitation and continue with available measurements.

## Final report contract

Every completed run should state:

1. Mode, scope, revision, workload, and machine/build context.
2. Which approved measurement tools were used, skipped, or unavailable, and why.
3. Before/after latency, throughput, CPU, allocation, heap, and peak RSS measurements **where actually collected**.
4. Statistical uncertainty and environmental limitations.
5. Correctness-check outcome and production-isolation verification.
6. Accepted/rejected optimizations, remaining bottlenecks, and exact paths to preserved results.

Never invent missing metrics, silently expand the approved tool list, or claim that the entire application has been covered when only selected code paths were exercised.
