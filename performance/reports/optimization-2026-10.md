# Rust workspace optimization report

**Verdict: verified improvements in selected workloads; otherwise a mixed, measurement-limited result.** Two full API verification runs and the final full workspace run confirmed large reductions in realistic host grouping and CPU fallback. Engine batch application also improved; JSON batch parsing remained within the baseline's broad variation. No global optimization claim is made.

## Source, candidate, and environment

- Baseline: `workspace-clean-idle-six-reps-20261012` from runs `20261012T060254851Z_workspace, 20261012T062250790Z_workspace`; 40 workloads across six repetitions.
- Candidate full run: `20261012T073036056Z_workspace`; 40 workloads across 3 repetitions.
- Additional API confirmation runs: `20261012T064523350Z_workspace`, `20261012T070457608Z_workspace` (the final run above also remeasured the API cases).
- Base and candidate source Git SHA: `b2ab3b53ef8dbe68cdef9c6f9de18173cde23971`. Candidate production sources are modified in the worktree; no Git state was changed.
- Compatibility key matched: `workspace-criterion-v1|x86_64-pc-windows-msvc|rustc 1.98.1 (48a229cea 2026-09-01)|bench|defaults|lock-7916af49e5073df7c73aa2a57829cfd42d37593eabea06d415a182622c6146c5`. Workload IDs, benchmark harness hashes, toolchain, target, and Cargo.lock hash match.
- 13th Gen Intel(R) Core(TM) i9-13980HX; Microsoft Windows 10.0.26200 ; rustc rustc 1.98.1 (48a229cea 2026-09-01); Criterion 0.8; 30 samples, 3-second warmup, 5-second measurement.
- Baseline variability is material: median six-run range across workloads is about 15%; per-case ranges are shown below. Small percentage changes are not treated as gains.

## Before and after

Improvement is `(before − after) / before`; positive values mean lower latency. Each row measures the full deterministic batch/operation, not per record.

| Workload | Before (ns) | Before range | After (ns) | After range | Change |
|---|---:|---:|---:|---:|---:|
| `criterion:actions/evaluate_policy.v1` | 282.88 | 70.8% | 248.71 | 4.0% | +12.08% |
| `criterion:actions/validate_scale.v1` | 21.38 | 7.9% | 22.00 | 1.6% | -2.91% |
| `criterion:api/group_500_hosts_and_containers.v1` | 265131.02 | 8.0% | 263391.01 | 2.9% | +0.66% |
| `criterion:api/group_500_hosts_cpu_utilization_fallback.v1` | 227730.97 | 25.6% | 152597.25 | 3.1% | +32.99% |
| `criterion:api/group_500_hosts_realistic_metrics.v1` | 1496950.42 | 11.9% | 963775.80 | 28.8% | +35.62% |
| `criterion:api/place_200_resources_600_addresses.v1` | 1679500.55 | 16.3% | 1675782.82 | 3.9% | +0.22% |
| `criterion:archive/decode_512_logs.v1` | 347398.19 | 13.5% | 348815.75 | 1.5% | -0.41% |
| `criterion:archive/encode_512_logs.v1` | 311250.17 | 16.0% | 318398.98 | 2.8% | -2.30% |
| `criterion:archive/index_512_logs.v1` | 83578.79 | 14.0% | 83427.69 | 2.1% | +0.18% |
| `criterion:archiver/plan_closed_windows.v1` | 428.87 | 21.3% | 420.22 | 2.4% | +2.02% |
| `criterion:cli/parse_node_entry.v1` | 107.70 | 22.9% | 105.26 | 3.9% | +2.26% |
| `criterion:convert/normalize_1000_logs.v1` | 1158822.94 | 15.9% | 1143530.87 | 4.0% | +1.32% |
| `criterion:convert/normalize_1000_spans.v1` | 954472.07 | 17.5% | 950108.99 | 1.4% | +0.46% |
| `criterion:convert/normalize_256_custom_events.v1` | 57776.13 | 22.8% | 56066.44 | 5.4% | +2.96% |
| `criterion:convert/normalize_256_metric_points.v1` | 304690.06 | 16.0% | 302946.60 | 4.1% | +0.57% |
| `criterion:domain/classify_common_attribute.v1` | 353.88 | 11.9% | 344.80 | 0.6% | +2.57% |
| `criterion:domain/classify_header.v1` | 49.47 | 11.7% | 47.64 | 2.0% | +3.71% |
| `criterion:domain/derive_scoped_service_id.v1` | 199.28 | 15.0% | 193.52 | 4.4% | +2.89% |
| `criterion:domain/mask_bearer_plain_value.v1` | 51.99 | 13.9% | 50.26 | 3.2% | +3.33% |
| `criterion:domain/mask_bearer_secret_value.v1` | 72.63 | 17.0% | 69.56 | 4.7% | +4.22% |
| `criterion:domain/redact_url_attribute.v1` | 120.79 | 15.5% | 118.81 | 5.0% | +1.64% |
| `criterion:domain/sanitize_common_string_attribute.v1` | 33.61 | 17.1% | 32.79 | 2.5% | +2.43% |
| `criterion:domain/sanitize_common_string_attribute_in_place.v1` | 12.60 | 11.2% | 12.40 | 2.9% | +1.60% |
| `criterion:engine/apply_batch_2000_spans.v1` | 758957.48 | 17.1% | 624481.18 | 2.6% | +17.72% |
| `criterion:engine/apply_batch_json_2000_spans.v1` | 2275559.03 | 14.0% | 2160761.47 | 1.7% | +5.04% |
| `criterion:engine/build_server_geometry_100_nodes.v1` | 869025.41 | 28.8% | 813741.31 | 1.7% | +6.36% |
| `criterion:engine/search_visible_state_1000_nodes.v1` | 23685.05 | 13.4% | 22609.42 | 7.6% | +4.54% |
| `criterion:engine/seek_and_build_visible_state.v1` | 23198.93 | 18.7% | 23220.14 | 2.6% | -0.09% |
| `criterion:executor/evaluate_and_record_verification.v1` | 1303.45 | 15.6% | 1256.43 | 7.6% | +3.61% |
| `criterion:ingest/sanitize_1000_logs_with_attributes.v1` | 575954.54 | 5.1% | 569101.17 | 1.5% | +1.19% |
| `criterion:ingest/sanitize_1000_metric_points_with_attributes.v1` | 865767.39 | 4.3% | 858169.62 | 0.7% | +0.88% |
| `criterion:ingest/sanitize_1000_traces.v1` | 43117.98 | 21.5% | 41644.17 | 11.2% | +3.42% |
| `criterion:ingest/sanitize_1000_traces_with_attributes.v1` | 730089.03 | 17.0% | 701545.47 | 2.4% | +3.91% |
| `criterion:processor/check_dead_letter_payload_bound.v1` | 0.57 | 11.2% | 0.48 | 32.2% | +15.18% |
| `criterion:processor/drain_128_ready_records.v1` | 203.41 | 5.1% | 197.86 | 3.0% | +2.73% |
| `criterion:protocol/decode_256_events.v1` | 27384.40 | 6.4% | 26608.42 | 1.2% | +2.83% |
| `criterion:protocol/encode_256_events.v1` | 5654.50 | 3.6% | 5429.94 | 3.4% | +3.97% |
| `criterion:runtime/backoff_schedule.v1` | 14.33 | 4.5% | 14.29 | 4.5% | +0.30% |
| `criterion:storage/parse_clickhouse_ttl.v1` | 155.70 | 5.1% | 156.70 | 6.1% | -0.64% |
| `criterion:web_engine/construct_from_config_json.v1` | 215.07 | 6.0% | 225.50 | 3.8% | -4.85% |

## Accepted changes

- `crates/karbird-api/src/routes/infrastructure.rs`: use hash maps to gather host and container series, then sort IDs to preserve deterministic output. In the final combined full run, realistic 500-host grouping improved 35.6% and CPU utilization fallback improved 33.0%. Across the two API-only confirmation runs, the corresponding means improved 43.4% and 33.7%, with between-run ranges of 2.5% and 1.1%. Plain host grouping was +0.7% faster in the final run; resource placement was +0.2%, both treated as neutral.
- `crates/karbird-engine-core/src/lib.rs`: count span/log/event items during the existing validation pass and reserve their maps when empty, avoiding repeated hash-table growth for an initial batch. `apply_batch` improved 17.7% in the final full run, with a 2.6% candidate range. The baseline range was 17.1%; the six underlying baseline repetitions ranged from 715–845 µs, while candidate repetitions were lower. `apply_batch_json` changed by +5.0%, within its 14.0% baseline range, so that end-to-end path is inconclusive.

## Rejected experiment

- Reusing a sorted index buffer for log fingerprint attributes slowed `convert/normalize_1000_logs` by 11.5% (Criterion interval +10.5% to +12.8%, p<0.05). The code was reverted; the raw experiment is preserved in `runs/20261012T0805_convert-log-index-experiment/`.

## Correctness and limits

- `just check`: passed (fmt, Clippy with warnings denied, wasm32, TypeScript). ESLint emitted three existing warnings in untouched `SignalsPanel.tsx`.
- `just test`: all Rust workspace tests and Node SDK tests passed; recipe exited 1 because Composer is not installed for PHP SDK tests. Focused API tests: 209 passed; engine-core: 171 passed; convert: 36 passed.
- Integration/e2e were not run. `just dev` could not connect because PostgreSQL was stopped. `just bootstrap` started local infrastructure but stopped at `migration 9 was previously applied but has been modified`; the database was not reset or repaired. The containers started during this check were stopped afterward.
- CPU profiling was attempted with installed Windows Performance Recorder but could not enable system profiling policy (`0xc5585011`); no profile or RSS data was collected. Kafka/database/S3 latency, browser rendering, allocation counts, RSS, and hardware counters remain unmeasured.
- The failed composer, bootstrap, and profiler attempts are recorded honestly; no tests or benchmark runs are reported as passed unless their output confirms success.

## Evidence and reproduction

- Baseline details: [`baseline.md`](baseline.md).
- Candidate comparison: [`20261012T073036056Z_workspace`](../runs/20261012T073036056Z_workspace/metadata.json) and its uniquely named comparison report under `comparisons/`.
- Re-run: `powershell -NoProfile -ExecutionPolicy Bypass -File performance/scripts/run-benchmarks.ps1 -Mode optimize`.
