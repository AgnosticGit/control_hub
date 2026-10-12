# Initial performance baseline

**Status: PARTIAL.** The baseline workload runner, immutable snapshot, manifests, and comparison checks work. The measurements are useful as a same-machine Rust CPU baseline, but they are not a complete application baseline: Git commit/dirty state cannot be recorded under the repository's agent policy, and service-level latency/throughput and memory are not measured.

## Source and environment

- Snapshot source run: `20261012T031450104Z_windows`; repeated run: `20261012T031539183Z_windows`.
- Source Git SHA: unavailable; working-tree state is unknown. This snapshot predates the current read-only Git policy, so it cannot be bound to a clean committed revision. Future runs record SHA and dirty status with read-only Git commands; this initial snapshot stays immutable.
- Cargo.lock SHA-256: `7916af49e5073df7c73aa2a57829cfd42d37593eabea06d415a182622c6146c5`.
- Windows 10.0.26200, x86_64 MSVC; Intel Core i9-13980HX, 32 logical processors, 34,116,345,856 bytes RAM.
- rustc 1.98.1 (LLVM 22.1.8), Cargo 1.98.1; default optimized Cargo bench profile, default features, system allocator.
- Criterion 0.8, 10 samples per case, 1-second warmup, 2-second measurement window. Each row is latency per whole batch/segment. The two run means show between-run variability as range divided by their mean; ± values show Criterion's estimated within-run standard deviation.

## Measured workloads

| Workload | Run 1 mean ± SD (ns) | Run 2 mean ± SD (ns) | Between-run range (% of mean) |
|---|---:|---:|---:|
| `ingest.sanitize-traces-1000.v1` — sanitize 1,000 synthetic spans with secret-bearing attributes | 947,803 ± 199,531 | 928,727 ± 167,819 | 2.0% |
| `engine.apply-batch-2000.v1` — apply 2,000 synthetic spans | 854,432 ± 70,712 | 890,066 ± 150,017 | 4.1% |
| `archive.encode-logs-512.v1` — encode 512 synthetic log records | 352,957 ± 50,952 | 333,370 ± 27,118 | 5.7% |

Inputs are deterministic synthetic fixtures defined by the existing benchmark harnesses. Criterion setup, including fixture construction and cloning via `iter_batched`, is excluded where documented in each workload spec. These figures measure individual CPU transformations; they do not represent a complete user workflow or production telemetry.

## Validation and limitations

- `cargo test --workspace`: passed.
- `just check`: passed. ESLint emitted three existing warnings in untouched `apps/web/src/components/workspace/SignalsPanel.tsx` (unused imports/state); no errors.
- `just test`: Rust workspace and Node SDK tests passed, then stopped because Composer is not installed (`composer` command unavailable). The PHP SDK tests therefore remain unverified.
- `verify-layout.py`: passed; required tree and checksums for baseline and all run manifests validated.
- `compare.py baseline:initial 20261012T031539183Z_windows`: passed and wrote a comparison report.
- Compatibility smoke check: a temporary run with workload version `v999` was refused as expected and removed afterward.
- The runner created separate, persistent run directories on repeated invocations. Two initial partial runs are retained with logs: `20261012T031233358Z_windows` (PowerShell treated Cargo's compiler output as an error) and `20261012T031307209Z_windows` (initial summary parser did not match Criterion's directory naming). The runner fixes are included in later successful runs.
- Peak RSS, heap allocations, CPU profiles, hardware counters, end-to-end ingest/API throughput and latency, database/Kafka effects, and WASM/browser rendering are **unavailable**. The current Windows host did not have a configured profiler or live-stack benchmark harness.
- Criterion output mentioned comparisons to pre-existing `target/criterion` data. The captured absolute estimates above are used; the tool's historical comparison messages were not treated as baseline deltas.

## Artifact paths

- Immutable first verified snapshot: `performance/baseline/initial/` (source run `20261012T031450104Z_windows`).
- Latest successful repeated run: `performance/runs/20261012T031539183Z_windows/`.
- Comparison report: `performance/reports/comparisons/comparison_20261012T031728Z_baseline-initial_20261012T031539183Z_windows.md`.
- Integrity checker: `performance/scripts/verify-layout.py`.

Re-run with `powershell -NoProfile -ExecutionPolicy Bypass -File performance/scripts/run-benchmarks.ps1`, then validate with `py -3 performance/scripts/verify-layout.py` and compare with `py -3 performance/scripts/compare.py baseline:initial <run-id>`.

No production algorithm was optimized or refactored for this baseline task.
