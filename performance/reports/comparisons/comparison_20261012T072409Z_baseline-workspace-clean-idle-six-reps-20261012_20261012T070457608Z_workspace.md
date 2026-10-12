# Performance comparison: baseline:workspace-clean-idle-six-reps-20261012 → 20261012T070457608Z_workspace

Compatibility key: `workspace-criterion-v1|x86_64-pc-windows-msvc|rustc 1.98.1 (48a229cea 2026-09-01)|bench|defaults|lock-7916af49e5073df7c73aa2a57829cfd42d37593eabea06d415a182622c6146c5`

| Workload | Before mean (ns) | After mean (ns) | Change | Before within-run SD | After within-run SD | Before run range | After run range |
|---|---:|---:|---:|---:|---:|---:|---:|
| criterion:actions/evaluate_policy.v1 | 282.879 | 242.351 | +14.33% | 6.502 | 2.917 | 70.75% | 3.42% |
| criterion:actions/validate_scale.v1 | 21.375 | 21.205 | +0.80% | 0.438 | 0.559 | 7.87% | 3.01% |
| criterion:api/group_500_hosts_and_containers.v1 | 265131.021 | 266131.524 | -0.38% | 5195.924 | 4145.225 | 8.00% | 13.12% |
| criterion:api/group_500_hosts_cpu_utilization_fallback.v1 | 227730.974 | 150088.274 | +34.09% | 9937.905 | 2102.489 | 25.57% | 1.67% |
| criterion:api/group_500_hosts_realistic_metrics.v1 | 1496950.417 | 837533.499 | +44.05% | 40782.053 | 16821.540 | 11.89% | 2.56% |
| criterion:api/place_200_resources_600_addresses.v1 | 1679500.546 | 1637526.669 | +2.50% | 69477.385 | 27675.019 | 16.28% | 2.89% |
| criterion:archive/decode_512_logs.v1 | 347398.192 | 340527.414 | +1.98% | 8726.195 | 6133.257 | 13.53% | 1.39% |
| criterion:archive/encode_512_logs.v1 | 311250.172 | 312407.732 | -0.37% | 15139.340 | 31322.391 | 16.00% | 2.72% |
| criterion:archive/index_512_logs.v1 | 83578.791 | 81446.523 | +2.55% | 1923.956 | 1336.469 | 14.05% | 2.54% |
| criterion:archiver/plan_closed_windows.v1 | 428.869 | 410.063 | +4.38% | 14.382 | 6.804 | 21.34% | 2.33% |
| criterion:cli/parse_node_entry.v1 | 107.701 | 105.070 | +2.44% | 5.515 | 1.445 | 22.86% | 3.90% |
| criterion:convert/normalize_1000_logs.v1 | 1158822.941 | 1124395.390 | +2.97% | 28738.693 | 18296.966 | 15.94% | 1.81% |
| criterion:convert/normalize_1000_spans.v1 | 954472.072 | 922746.828 | +3.32% | 29076.714 | 12350.120 | 17.50% | 4.39% |
| criterion:convert/normalize_256_custom_events.v1 | 57776.128 | 55662.028 | +3.66% | 2609.337 | 1402.242 | 22.81% | 1.89% |
| criterion:convert/normalize_256_metric_points.v1 | 304690.063 | 296441.387 | +2.71% | 10482.730 | 4515.658 | 16.03% | 4.15% |
| criterion:domain/classify_common_attribute.v1 | 353.885 | 352.863 | +0.29% | 11.260 | 4.341 | 11.86% | 0.32% |
| criterion:domain/classify_header.v1 | 49.472 | 49.873 | -0.81% | 1.740 | 1.060 | 11.70% | 3.85% |
| criterion:domain/derive_scoped_service_id.v1 | 199.281 | 196.211 | +1.54% | 5.628 | 3.001 | 14.96% | 4.46% |
| criterion:domain/mask_bearer_plain_value.v1 | 51.993 | 51.383 | +1.17% | 2.308 | 0.848 | 13.90% | 3.35% |
| criterion:domain/mask_bearer_secret_value.v1 | 72.625 | 71.510 | +1.54% | 2.420 | 1.282 | 17.00% | 2.60% |
| criterion:domain/redact_url_attribute.v1 | 120.788 | 120.229 | +0.46% | 4.697 | 1.678 | 15.46% | 2.70% |
| criterion:domain/sanitize_common_string_attribute.v1 | 33.606 | 33.841 | -0.70% | 1.119 | 1.245 | 17.05% | 4.69% |
| criterion:domain/sanitize_common_string_attribute_in_place.v1 | 12.601 | 12.589 | +0.10% | 0.485 | 0.420 | 11.18% | 2.05% |
| criterion:engine/apply_batch_2000_spans.v1 | 758957.475 | 749202.981 | +1.29% | 30097.815 | 31845.185 | 17.06% | 2.27% |
| criterion:engine/apply_batch_json_2000_spans.v1 | 2275559.034 | 2260094.051 | +0.68% | 71759.151 | 32249.122 | 14.04% | 2.15% |
| criterion:engine/build_server_geometry_100_nodes.v1 | 869025.407 | 826962.129 | +4.84% | 23571.701 | 12790.617 | 28.81% | 3.07% |
| criterion:engine/search_visible_state_1000_nodes.v1 | 23685.055 | 23358.009 | +1.38% | 770.033 | 434.090 | 13.43% | 5.26% |
| criterion:engine/seek_and_build_visible_state.v1 | 23198.935 | 23070.884 | +0.55% | 848.358 | 486.751 | 18.70% | 1.91% |
| criterion:executor/evaluate_and_record_verification.v1 | 1303.449 | 1282.083 | +1.64% | 35.185 | 16.175 | 15.60% | 3.44% |
| criterion:ingest/sanitize_1000_logs_with_attributes.v1 | 575954.536 | 569446.417 | +1.13% | 13140.883 | 11137.911 | 5.07% | 3.78% |
| criterion:ingest/sanitize_1000_metric_points_with_attributes.v1 | 865767.390 | 855801.143 | +1.15% | 18454.563 | 16790.669 | 4.29% | 4.14% |
| criterion:ingest/sanitize_1000_traces.v1 | 43117.983 | 43083.248 | +0.08% | 7652.687 | 9955.106 | 21.53% | 8.73% |
| criterion:ingest/sanitize_1000_traces_with_attributes.v1 | 730089.034 | 713559.528 | +2.26% | 30152.063 | 17028.118 | 17.02% | 2.44% |
| criterion:processor/check_dead_letter_payload_bound.v1 | 0.566 | 0.432 | +23.63% | 0.191 | 0.085 | 11.18% | 36.67% |
| criterion:processor/drain_128_ready_records.v1 | 203.415 | 198.651 | +2.34% | 23.031 | 24.185 | 5.07% | 5.65% |
| criterion:protocol/decode_256_events.v1 | 27384.397 | 26448.515 | +3.42% | 489.119 | 358.861 | 6.45% | 2.54% |
| criterion:protocol/encode_256_events.v1 | 5654.499 | 5472.483 | +3.22% | 93.158 | 110.213 | 3.59% | 2.35% |
| criterion:runtime/backoff_schedule.v1 | 14.330 | 14.126 | +1.43% | 0.223 | 0.181 | 4.51% | 1.82% |
| criterion:storage/parse_clickhouse_ttl.v1 | 155.705 | 152.561 | +2.02% | 2.142 | 2.277 | 5.13% | 2.98% |
| criterion:web_engine/construct_from_config_json.v1 | 215.065 | 210.968 | +1.91% | 2.778 | 3.075 | 5.98% | 2.45% |

Generated from stored Criterion summaries. Variation is represented by Criterion's estimated standard deviation; no significance claim is made by this report.
