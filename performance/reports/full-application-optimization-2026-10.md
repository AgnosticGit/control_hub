# Full application performance coverage and optimization — October 2026

## Scope and environment

This pass covered the complete existing Rust Criterion suite: 15 benchmark targets and 40 deterministic CPU workloads. The before run is `20261012T135755240Z_workspace`; the after run is `20261012T145454592Z_workspace`. Both use Rust 1.98.1, the same Windows x86_64 host, optimized bench profile, workload catalog and Cargo.lock compatibility key. Each has three repetitions and 30 Criterion samples per case. The machine is an Intel Core i9-13980HX with 32 logical processors and 34.1 GB RAM.

Additional tool coverage:

- Divan 0.1.21 allocation measurements cover eight representative paths across API, archive, convert, engine, ingest and protocol. The earlier set is `20261012T142120657Z_divan`; the final set is `20261012T152000000Z_divan-final`.
- Hyperfine 2.0.0 ran two native CLI repository workloads with three warmups and 20 samples. The initial run is `20261012T142400000Z_app-baseline`; the paired run is `20261012T151600000Z_app-paired`.
- Peak RSS was collected five times per CLI workload with Windows process peak working-set accounting. This is OS-accounted resident memory, in bytes.
- Linux `perf` was unavailable because the host is Windows; WSL2 did not have `perf` installed and hardware-counter access was restricted. Gungraun was unavailable because Valgrind was not installed; the available WSL Valgrind package was below Gungraun's tested version and required elevated setup. No substitute profiler was introduced.

## Accepted optimization

`crates/karbird-api/src/routes/infrastructure.rs` previously collected temporary `Vec`s to determine whether a container match was unique, once for local matches and again for global matches. The resolution now counts matching entries in place and exits as soon as it finds ambiguity. The existing behavior remains: a unique local match wins, global matching is attempted only when there is no local match, and ambiguous matches are not guessed.

| Measurement | Before | After | Change |
|---|---:|---:|---:|
| Criterion: place 200 resources / 600 addresses, mean | 1.661 ms | 1.601 ms | 3.6% faster |
| Criterion: between-repetition range | 4.24% | 2.89% | lower variation |
| Divan allocations per call | 8,909 | 8,309 | 600 fewer (6.7%) |
| Divan allocated bytes per call | 479.7 KB | 460.5 KB | 19.2 KB fewer (4.0%) |

The Criterion means are the accepted native latency comparison from the complete three-repetition runs. The allocation counts and bytes come from the same 200-resource / 600-address workload with fixture setup outside the measured call. Divan's instrumented timing is not used as production latency evidence. The targeted run logs and metadata are in `runs/20261012T145000000Z_resource-placement-ab/`.

The full 40-workload comparison is [preserved here](comparisons/comparison_20261012T151410Z_20261012T135755240Z_workspace_20261012T145454592Z_workspace.md). Changes in unrelated microbenchmarks are not attributed to this API change; their individual run ranges and standard deviations are included in that report.

## Rejected experiment

The 1,000-log convert benchmark showed geometric growth of the output vector, so an exact capacity reservation was evaluated. Divan recorded 17 → 9 vector growth events, but total allocated bytes (alloc + grow) changed only from about 1,350.5 KB to 1,340.7 KB (−0.7%), with no allocation-count reduction. Criterion reported no detectable latency change (`p = 0.83`; estimate 1.124 ms, interval 1.119–1.130 ms). The production change was removed because the measurement did not support a useful improvement. The baseline and candidate logs are preserved in `runs/20261012T145000000Z_resource-placement-ab/`.

## Whole-process checks

The paired CLI Hyperfine means were 9.855 ms → 9.148 ms for `discover` and 7.469 ms → 9.841 ms for `plan`. These are small process-through-repository scenarios, not API request latency, and the samples varied substantially (standard deviations of 2.1–2.4 ms). They do not implicate the changed route and are not evidence of an application-wide speedup or regression.

Peak RSS stayed near 3.19 MB for both commands. The Windows process counter's granularity and the small workloads do not resolve a meaningful memory change at this scale.

## Verification and limits

- `cargo test --locked -p karbird-api --lib routes::infrastructure::tests::`: 28 passed.
- `just check`: passed, including fmt, workspace clippy on all targets, WASM check, TypeScript typecheck and ESLint. ESLint reported three pre-existing warnings in `SignalsPanel.tsx`, with no errors.
- `just test`: Rust workspace suites and Node SDK tests passed; the overall recipe exited 1 because Composer is missing, so PHP SDK tests could not run.
- Normal release build: `cargo build --locked --release -p karbird-cli` passed. `cargo tree --locked -p karbird-cli -e normal` showed no Criterion, Divan, Gungraun or Hyperfine dependency in the production dependency graph.
- API integration/e2e were not available: `just dev` started the web server, but Rust services failed to connect to PostgreSQL (pool timeout). `just stop` reported no remaining managed processes. The existing local database/bootstrap issue was not repaired or reset.

The measurement covers every existing Rust Criterion target plus selected allocation and CLI paths. It does not cover live Kafka/PostgreSQL/ClickHouse request latency, concurrent service throughput, browser rendering, or full-stack peak RSS. The supported tools and local environment do not provide those results in this run, so this report makes no claim that the entire application is globally optimized.

