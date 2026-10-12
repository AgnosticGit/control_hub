# Performance comparison: baseline:workspace-clean-idle-six-reps-20261012 → 20261012T064523350Z_workspace

Compatibility key: `workspace-criterion-v1|x86_64-pc-windows-msvc|rustc 1.98.1 (48a229cea 2026-09-01)|bench|defaults|lock-7916af49e5073df7c73aa2a57829cfd42d37593eabea06d415a182622c6146c5`

| Workload | Before mean (ns) | After mean (ns) | Change | Before within-run SD | After within-run SD | Before run range | After run range |
|---|---:|---:|---:|---:|---:|---:|---:|
| criterion:actions/evaluate_policy.v1 | 282.879 | 258.990 | +8.44% | 6.502 | 4.498 | 70.75% | 6.81% |
| criterion:actions/validate_scale.v1 | 21.375 | 22.622 | -5.83% | 0.438 | 1.153 | 7.87% | 13.32% |
| criterion:api/group_500_hosts_and_containers.v1 | 265131.021 | 272282.266 | -2.70% | 5195.924 | 6390.205 | 8.00% | 5.66% |
| criterion:api/group_500_hosts_cpu_utilization_fallback.v1 | 227730.974 | 151739.390 | +33.37% | 9937.905 | 2800.481 | 25.57% | 4.34% |
| criterion:api/group_500_hosts_realistic_metrics.v1 | 1496950.417 | 858448.066 | +42.65% | 40782.053 | 11468.912 | 11.89% | 5.91% |
| criterion:api/place_200_resources_600_addresses.v1 | 1679500.546 | 1648126.476 | +1.87% | 69477.385 | 26680.542 | 16.28% | 1.69% |
| criterion:archive/decode_512_logs.v1 | 347398.192 | 345106.956 | +0.66% | 8726.195 | 6265.654 | 13.53% | 4.94% |
| criterion:archive/encode_512_logs.v1 | 311250.172 | 312560.637 | -0.42% | 15139.340 | 32908.029 | 16.00% | 7.30% |
| criterion:archive/index_512_logs.v1 | 83578.791 | 82211.347 | +1.64% | 1923.956 | 1689.066 | 14.05% | 2.38% |
| criterion:archiver/plan_closed_windows.v1 | 428.869 | 469.694 | -9.52% | 14.382 | 10.063 | 21.34% | 33.65% |
| criterion:cli/parse_node_entry.v1 | 107.701 | 103.659 | +3.75% | 5.515 | 1.337 | 22.86% | 2.10% |
| criterion:convert/normalize_1000_logs.v1 | 1158822.941 | 1129871.271 | +2.50% | 28738.693 | 15027.624 | 15.94% | 6.02% |
| criterion:convert/normalize_1000_spans.v1 | 954472.072 | 936888.299 | +1.84% | 29076.714 | 23137.997 | 17.50% | 3.30% |
| criterion:convert/normalize_256_custom_events.v1 | 57776.128 | 55804.902 | +3.41% | 2609.337 | 877.875 | 22.81% | 3.57% |
| criterion:convert/normalize_256_metric_points.v1 | 304690.063 | 297251.613 | +2.44% | 10482.730 | 4191.070 | 16.03% | 0.76% |
| criterion:domain/classify_common_attribute.v1 | 353.885 | 345.923 | +2.25% | 11.260 | 4.738 | 11.86% | 1.61% |
| criterion:domain/classify_header.v1 | 49.472 | 48.345 | +2.28% | 1.740 | 1.106 | 11.70% | 5.34% |
| criterion:domain/derive_scoped_service_id.v1 | 199.281 | 191.121 | +4.09% | 5.628 | 2.291 | 14.96% | 6.60% |
| criterion:domain/mask_bearer_plain_value.v1 | 51.993 | 49.890 | +4.04% | 2.308 | 0.874 | 13.90% | 5.14% |
| criterion:domain/mask_bearer_secret_value.v1 | 72.625 | 112.770 | -55.28% | 2.420 | 3.093 | 17.00% | 110.64% |
| criterion:domain/redact_url_attribute.v1 | 120.788 | 120.834 | -0.04% | 4.697 | 1.917 | 15.46% | 2.14% |
| criterion:domain/sanitize_common_string_attribute.v1 | 33.606 | 32.994 | +1.82% | 1.119 | 0.448 | 17.05% | 3.76% |
| criterion:domain/sanitize_common_string_attribute_in_place.v1 | 12.601 | 12.354 | +1.96% | 0.485 | 0.450 | 11.18% | 0.17% |
| criterion:engine/apply_batch_2000_spans.v1 | 758957.475 | 719207.013 | +5.24% | 30097.815 | 20518.599 | 17.06% | 1.73% |
| criterion:engine/apply_batch_json_2000_spans.v1 | 2275559.034 | 2230273.027 | +1.99% | 71759.151 | 36343.914 | 14.04% | 3.58% |
| criterion:engine/build_server_geometry_100_nodes.v1 | 869025.407 | 829996.399 | +4.49% | 23571.701 | 11122.246 | 28.81% | 6.06% |
| criterion:engine/search_visible_state_1000_nodes.v1 | 23685.055 | 23526.411 | +0.67% | 770.033 | 393.051 | 13.43% | 2.37% |
| criterion:engine/seek_and_build_visible_state.v1 | 23198.935 | 21897.779 | +5.61% | 848.358 | 286.220 | 18.70% | 1.11% |
| criterion:executor/evaluate_and_record_verification.v1 | 1303.449 | 1293.399 | +0.77% | 35.185 | 16.083 | 15.60% | 9.60% |
| criterion:ingest/sanitize_1000_logs_with_attributes.v1 | 575954.536 | 585021.841 | -1.57% | 13140.883 | 13375.971 | 5.07% | 1.60% |
| criterion:ingest/sanitize_1000_metric_points_with_attributes.v1 | 865767.390 | 877820.976 | -1.39% | 18454.563 | 15304.340 | 4.29% | 2.65% |
| criterion:ingest/sanitize_1000_traces.v1 | 43117.983 | 43017.579 | +0.23% | 7652.687 | 11593.339 | 21.53% | 8.60% |
| criterion:ingest/sanitize_1000_traces_with_attributes.v1 | 730089.034 | 727841.834 | +0.31% | 30152.063 | 18173.933 | 17.02% | 1.55% |
| criterion:processor/check_dead_letter_payload_bound.v1 | 0.566 | 0.452 | +20.05% | 0.191 | 0.100 | 11.18% | 40.76% |
| criterion:processor/drain_128_ready_records.v1 | 203.415 | 201.200 | +1.09% | 23.031 | 20.866 | 5.07% | 5.71% |
| criterion:protocol/decode_256_events.v1 | 27384.397 | 28912.816 | -5.58% | 489.119 | 646.150 | 6.45% | 12.00% |
| criterion:protocol/encode_256_events.v1 | 5654.499 | 5924.461 | -4.77% | 93.158 | 140.804 | 3.59% | 14.02% |
| criterion:runtime/backoff_schedule.v1 | 14.330 | 15.238 | -6.33% | 0.223 | 0.210 | 4.51% | 12.59% |
| criterion:storage/parse_clickhouse_ttl.v1 | 155.705 | 167.875 | -7.82% | 2.142 | 3.923 | 5.13% | 15.29% |
| criterion:web_engine/construct_from_config_json.v1 | 215.065 | 233.287 | -8.47% | 2.778 | 5.931 | 5.98% | 15.90% |

Generated from stored Criterion summaries. Variation is represented by Criterion's estimated standard deviation; no significance claim is made by this report.
