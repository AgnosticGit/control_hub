# Performance measurement

This directory contains a persistent performance harness for the karbird Rust workspace. Baseline runs measure the full Criterion suite; optimize runs compare the same suite with the active immutable baseline.

## Host requirements

- Rust toolchain from `rust-toolchain.toml`, Cargo, and Python 3.
- Windows: PowerShell 7 (`pwsh`) or Windows PowerShell 5.1 and Python. Invoke with `powershell -ExecutionPolicy Bypass -File performance/scripts/run-benchmarks.ps1`.
- Linux/macOS: Bash and PowerShell 7. Invoke with `bash performance/scripts/run-benchmarks.sh`.
- A stable, otherwise idle machine. Do not run another benchmark or CPU-heavy workload concurrently.

The workspace catalog covers all 15 existing Rust Criterion targets and their 40 benchmark cases. Each case calls production CPU code; deterministic fixture construction and timed boundaries are defined by the source harness listed in `benchmarks/workloads/workspace-criterion-v1.json`. This covers the existing Rust benchmark suite, but does not claim to measure live-stack API/Kafka/ClickHouse or browser end-to-end latency.

## Commands

```powershell
# Correctness before establishing or comparing a baseline
cargo test --workspace

# Create a fresh full-suite baseline run (never overwrites an existing run)
powershell -NoProfile -ExecutionPolicy Bypass -File performance/scripts/run-benchmarks.ps1

# Compare a fresh full-suite run with the active baseline
powershell -NoProfile -ExecutionPolicy Bypass -File performance/scripts/run-benchmarks.ps1 -Mode optimize

# Validate artifact layout and checksums
py -3 performance/scripts/verify-layout.py

# Compare a new run with the clean workspace baseline
py -3 performance/scripts/compare.py baseline:<baseline-name> <run-id>

# Measure allocations on the representative CPU workloads
powershell -NoProfile -ExecutionPolicy Bypass -File performance/scripts/run-divan-suite.ps1

# Build the native CLI, then measure end-to-end commands and Peak RSS
cargo build --locked --release -p karbird-cli
powershell -NoProfile -ExecutionPolicy Bypass -File performance/scripts/run-app-workloads.ps1 `
  -RunId <unique-run-id> -Executable target/release/karbird.exe
```

The shell entrypoint `performance/scripts/run-benchmarks.sh` delegates to the PowerShell runner on Windows and PowerShell 7 elsewhere. One invocation performs three complete workspace repetitions, with 30 Criterion samples, a 3-second warmup, and a 5-second measurement window per case. Each run stores repetition logs, raw Criterion output, per-repetition and aggregate summaries, host/tool/Git metadata, source hashes, and a SHA-256 manifest. Every invocation creates a new run directory. The accumulated pre-existing `target/criterion` tree (134 estimate sets) is preserved in its own run archive. The active baseline combines six repetitions from two later full-suite runs after the machine was left idle; `reports/baseline.md` lists all 40 cases and their variation.

`run-divan-suite.ps1` measures allocation behavior in representative API, archive, convert, engine, ingest, and protocol CPU paths. `run-app-workloads.ps1` uses Hyperfine for the native CLI `discover` and `plan` flows and `measure-peak-rss.ps1` records OS-native process Peak RSS. Criterion is the latency comparison source; Divan's instrumented timings are not production latency evidence. Linux perf and Gungraun are only used where their platform/runtime prerequisites are met.

## Artifacts and retention

Small specs, configuration, scripts, normalized summaries, manifests, and reports are intended for version control. Criterion output may grow; retain it locally or in an artifact store and keep its manifest. Do not place secrets or real telemetry in fixtures. Run folders and every baseline folder are append-only. The three-case `baseline/initial/` remains as the historical partial baseline; the active clean baseline is named in `config.toml`. `merge-baseline-runs.py` creates a new immutable baseline from compatible runs without replacing earlier snapshots. The comparison tool refuses differences in compatibility key, workload IDs/versions, source harness hashes, or metric schema and preserves uniquely named reports.

The historical six-repetition baseline is documented in `reports/baseline.md`. The latest complete application measurement and optimization evidence, including Criterion, Divan, Hyperfine, Peak RSS, test outcomes and environment limits, are in `reports/full-application-optimization-2026-10.md`.

## Baseline caveats

The initial snapshot predates the read-only Git policy and remains unchanged. New runs record the commit SHA and dirty status. The Criterion suite is the full set of existing Rust CPU microbenchmarks. Allocation, RSS, hardware counters, CPU profiles, and live application/browser end-to-end performance are separate measurements and must not be inferred from Criterion results.
