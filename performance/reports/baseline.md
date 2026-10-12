# Clean Rust workspace performance baseline

**Status: COMPLETE for the existing Rust Criterion suite.** Baseline `workspace-clean-idle-six-reps-20261012` covers 40 workloads across 6 repetitions from two independent full-suite runs.

## Source and environment

- Source Git SHA: `b2ab3b53ef8dbe68cdef9c6f9de18173cde23971`; production Rust sources were clean (`production_sources_dirty=false).
- Source runs: `20261012T060254851Z_workspace`, `20261012T062250790Z_workspace`.
- Cargo.lock SHA-256: `7916af49e5073df7c73aa2a57829cfd42d37593eabea06d415a182622c6146c5`; compatibility key: `workspace-criterion-v1|x86_64-pc-windows-msvc|rustc 1.98.1 (48a229cea 2026-09-01)|bench|defaults|lock-7916af49e5073df7c73aa2a57829cfd42d37593eabea06d415a182622c6146c5`.
- Microsoft Windows 10.0.26200 , X64; 13th Gen Intel(R) Core(TM) i9-13980HX; 32 logical processors; 34,116,345,856 bytes RAM.
- rustc 1.98.1 (48a229cea 2026-09-01); cargo 1.98.1 (797e8a9bc 2026-08-05); optimized Cargo bench profile; workspace default features; system allocator.
- Criterion 0.8; 30 samples, 3-second warmup, 5-second measurement per case; deterministic fixed benchmark fixtures.
- Across these six repetitions, per-workload between-run range has a median of 15.0% and a 90th percentile of 22.8%. Use this spread when judging small changes.

## All measured workloads

Each time is the mean cost of the entire benchmark operation, not per item. Between-run range is `(max repetition mean − min repetition mean) / aggregate mean`.

| Workload | Operation | Mean (ns) | Criterion SD (ns) | Six-run range |
|---|---|---:|---:|---:|
| `criterion:actions/evaluate_policy.v1` | Existing deterministic CPU workload calling production code: actions/evaluate_policy. Fixture and measured boundary are defined in the referenced Criterion harness. | 282.88 | 6.50 | 70.8% |
| `criterion:actions/validate_scale.v1` | Existing deterministic CPU workload calling production code: actions/validate_scale. Fixture and measured boundary are defined in the referenced Criterion harness. | 21.38 | 0.44 | 7.9% |
| `criterion:api/group_500_hosts_and_containers.v1` | Existing deterministic CPU workload calling production code: api/group_500_hosts_and_containers. Fixture and measured boundary are defined in the referenced Criterion harness. | 265131.02 | 5195.92 | 8.0% |
| `criterion:api/group_500_hosts_cpu_utilization_fallback.v1` | Existing deterministic CPU workload calling production code: api/group_500_hosts_cpu_utilization_fallback. Fixture and measured boundary are defined in the referenced Criterion harness. | 227730.97 | 9937.90 | 25.6% |
| `criterion:api/group_500_hosts_realistic_metrics.v1` | Existing deterministic CPU workload calling production code: api/group_500_hosts_realistic_metrics. Fixture and measured boundary are defined in the referenced Criterion harness. | 1496950.42 | 40782.05 | 11.9% |
| `criterion:api/place_200_resources_600_addresses.v1` | Existing deterministic CPU workload calling production code: api/place_200_resources_600_addresses. Fixture and measured boundary are defined in the referenced Criterion harness. | 1679500.55 | 69477.38 | 16.3% |
| `criterion:archive/decode_512_logs.v1` | Existing deterministic CPU workload calling production code: archive/decode_512_logs. Fixture and measured boundary are defined in the referenced Criterion harness. | 347398.19 | 8726.20 | 13.5% |
| `criterion:archive/encode_512_logs.v1` | Existing deterministic CPU workload calling production code: archive/encode_512_logs. Fixture and measured boundary are defined in the referenced Criterion harness. | 311250.17 | 15139.34 | 16.0% |
| `criterion:archive/index_512_logs.v1` | Existing deterministic CPU workload calling production code: archive/index_512_logs. Fixture and measured boundary are defined in the referenced Criterion harness. | 83578.79 | 1923.96 | 14.0% |
| `criterion:archiver/plan_closed_windows.v1` | Existing deterministic CPU workload calling production code: archiver/plan_closed_windows. Fixture and measured boundary are defined in the referenced Criterion harness. | 428.87 | 14.38 | 21.3% |
| `criterion:cli/parse_node_entry.v1` | Existing deterministic CPU workload calling production code: cli/parse_node_entry. Fixture and measured boundary are defined in the referenced Criterion harness. | 107.70 | 5.51 | 22.9% |
| `criterion:convert/normalize_1000_logs.v1` | Existing deterministic CPU workload calling production code: convert/normalize_1000_logs. Fixture and measured boundary are defined in the referenced Criterion harness. | 1158822.94 | 28738.69 | 15.9% |
| `criterion:convert/normalize_1000_spans.v1` | Existing deterministic CPU workload calling production code: convert/normalize_1000_spans. Fixture and measured boundary are defined in the referenced Criterion harness. | 954472.07 | 29076.71 | 17.5% |
| `criterion:convert/normalize_256_custom_events.v1` | Existing deterministic CPU workload calling production code: convert/normalize_256_custom_events. Fixture and measured boundary are defined in the referenced Criterion harness. | 57776.13 | 2609.34 | 22.8% |
| `criterion:convert/normalize_256_metric_points.v1` | Existing deterministic CPU workload calling production code: convert/normalize_256_metric_points. Fixture and measured boundary are defined in the referenced Criterion harness. | 304690.06 | 10482.73 | 16.0% |
| `criterion:domain/classify_common_attribute.v1` | Existing deterministic CPU workload calling production code: domain/classify_common_attribute. Fixture and measured boundary are defined in the referenced Criterion harness. | 353.88 | 11.26 | 11.9% |
| `criterion:domain/classify_header.v1` | Existing deterministic CPU workload calling production code: domain/classify_header. Fixture and measured boundary are defined in the referenced Criterion harness. | 49.47 | 1.74 | 11.7% |
| `criterion:domain/derive_scoped_service_id.v1` | Existing deterministic CPU workload calling production code: domain/derive_scoped_service_id. Fixture and measured boundary are defined in the referenced Criterion harness. | 199.28 | 5.63 | 15.0% |
| `criterion:domain/mask_bearer_plain_value.v1` | Existing deterministic CPU workload calling production code: domain/mask_bearer_plain_value. Fixture and measured boundary are defined in the referenced Criterion harness. | 51.99 | 2.31 | 13.9% |
| `criterion:domain/mask_bearer_secret_value.v1` | Existing deterministic CPU workload calling production code: domain/mask_bearer_secret_value. Fixture and measured boundary are defined in the referenced Criterion harness. | 72.63 | 2.42 | 17.0% |
| `criterion:domain/redact_url_attribute.v1` | Existing deterministic CPU workload calling production code: domain/redact_url_attribute. Fixture and measured boundary are defined in the referenced Criterion harness. | 120.79 | 4.70 | 15.5% |
| `criterion:domain/sanitize_common_string_attribute.v1` | Existing deterministic CPU workload calling production code: domain/sanitize_common_string_attribute. Fixture and measured boundary are defined in the referenced Criterion harness. | 33.61 | 1.12 | 17.1% |
| `criterion:domain/sanitize_common_string_attribute_in_place.v1` | Existing deterministic CPU workload calling production code: domain/sanitize_common_string_attribute_in_place. Fixture and measured boundary are defined in the referenced Criterion harness. | 12.60 | 0.48 | 11.2% |
| `criterion:engine/apply_batch_2000_spans.v1` | Existing deterministic CPU workload calling production code: engine/apply_batch_2000_spans. Fixture and measured boundary are defined in the referenced Criterion harness. | 758957.48 | 30097.81 | 17.1% |
| `criterion:engine/apply_batch_json_2000_spans.v1` | Existing deterministic CPU workload calling production code: engine/apply_batch_json_2000_spans. Fixture and measured boundary are defined in the referenced Criterion harness. | 2275559.03 | 71759.15 | 14.0% |
| `criterion:engine/build_server_geometry_100_nodes.v1` | Existing deterministic CPU workload calling production code: engine/build_server_geometry_100_nodes. Fixture and measured boundary are defined in the referenced Criterion harness. | 869025.41 | 23571.70 | 28.8% |
| `criterion:engine/search_visible_state_1000_nodes.v1` | Existing deterministic CPU workload calling production code: engine/search_visible_state_1000_nodes. Fixture and measured boundary are defined in the referenced Criterion harness. | 23685.05 | 770.03 | 13.4% |
| `criterion:engine/seek_and_build_visible_state.v1` | Existing deterministic CPU workload calling production code: engine/seek_and_build_visible_state. Fixture and measured boundary are defined in the referenced Criterion harness. | 23198.93 | 848.36 | 18.7% |
| `criterion:executor/evaluate_and_record_verification.v1` | Existing deterministic CPU workload calling production code: executor/evaluate_and_record_verification. Fixture and measured boundary are defined in the referenced Criterion harness. | 1303.45 | 35.19 | 15.6% |
| `criterion:ingest/sanitize_1000_logs_with_attributes.v1` | Existing deterministic CPU workload calling production code: ingest/sanitize_1000_logs_with_attributes. Fixture and measured boundary are defined in the referenced Criterion harness. | 575954.54 | 13140.88 | 5.1% |
| `criterion:ingest/sanitize_1000_metric_points_with_attributes.v1` | Existing deterministic CPU workload calling production code: ingest/sanitize_1000_metric_points_with_attributes. Fixture and measured boundary are defined in the referenced Criterion harness. | 865767.39 | 18454.56 | 4.3% |
| `criterion:ingest/sanitize_1000_traces.v1` | Existing deterministic CPU workload calling production code: ingest/sanitize_1000_traces. Fixture and measured boundary are defined in the referenced Criterion harness. | 43117.98 | 7652.69 | 21.5% |
| `criterion:ingest/sanitize_1000_traces_with_attributes.v1` | Existing deterministic CPU workload calling production code: ingest/sanitize_1000_traces_with_attributes. Fixture and measured boundary are defined in the referenced Criterion harness. | 730089.03 | 30152.06 | 17.0% |
| `criterion:processor/check_dead_letter_payload_bound.v1` | Existing deterministic CPU workload calling production code: processor/check_dead_letter_payload_bound. Fixture and measured boundary are defined in the referenced Criterion harness. | 0.57 | 0.19 | 11.2% |
| `criterion:processor/drain_128_ready_records.v1` | Existing deterministic CPU workload calling production code: processor/drain_128_ready_records. Fixture and measured boundary are defined in the referenced Criterion harness. | 203.41 | 23.03 | 5.1% |
| `criterion:protocol/decode_256_events.v1` | Existing deterministic CPU workload calling production code: protocol/decode_256_events. Fixture and measured boundary are defined in the referenced Criterion harness. | 27384.40 | 489.12 | 6.4% |
| `criterion:protocol/encode_256_events.v1` | Existing deterministic CPU workload calling production code: protocol/encode_256_events. Fixture and measured boundary are defined in the referenced Criterion harness. | 5654.50 | 93.16 | 3.6% |
| `criterion:runtime/backoff_schedule.v1` | Existing deterministic CPU workload calling production code: runtime/backoff_schedule. Fixture and measured boundary are defined in the referenced Criterion harness. | 14.33 | 0.22 | 4.5% |
| `criterion:storage/parse_clickhouse_ttl.v1` | Existing deterministic CPU workload calling production code: storage/parse_clickhouse_ttl. Fixture and measured boundary are defined in the referenced Criterion harness. | 155.70 | 2.14 | 5.1% |
| `criterion:web_engine/construct_from_config_json.v1` | Existing deterministic CPU workload calling production code: web_engine/construct_from_config_json. Fixture and measured boundary are defined in the referenced Criterion harness. | 215.07 | 2.78 | 6.0% |

## Historical measurements and limits

- `baseline/initial/` and `reports/baseline-initial-partial.md` preserve the original three-case baseline unchanged.
- `runs/historic-target-criterion_20261012T033203378Z/` preserves 134 existing Criterion estimate sets and the 40-case prior `new` set. They were collected under different conditions and are supplementary historical context, not mixed into this clean baseline.
- The full Rust CPU microbenchmark suite covers 15 existing benchmark targets across domain, API, archive, archiver, CLI, convert, engine, executor, ingest, processor, protocol, runtime, storage, and web-engine crates.
- This suite does not measure live Kafka/PostgreSQL/ClickHouse/S3 latency, full service throughput, browser rendering, peak RSS, allocation counts, hardware counters, or CPU profiles. Do not infer those values from Criterion CPU timings.
- The first full run showed substantially higher variation than the later idle runs; it is preserved as calibration history and not included in the active baseline.

Generated 2026-10-12T06:42:25.632960+00:00 from the immutable baseline artifacts.
