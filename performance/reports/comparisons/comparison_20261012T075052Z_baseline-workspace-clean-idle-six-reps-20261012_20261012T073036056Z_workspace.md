# Performance comparison: baseline:workspace-clean-idle-six-reps-20261012 → 20261012T073036056Z_workspace

Compatibility key: `workspace-criterion-v1|x86_64-pc-windows-msvc|rustc 1.98.1 (48a229cea 2026-09-01)|bench|defaults|lock-7916af49e5073df7c73aa2a57829cfd42d37593eabea06d415a182622c6146c5`

| Workload | Before mean (ns) | After mean (ns) | Change | Before within-run SD | After within-run SD | Before run range | After run range |
|---|---:|---:|---:|---:|---:|---:|---:|
| criterion:actions/evaluate_policy.v1 | 282.879 | 248.714 | +12.08% | 6.502 | 4.096 | 70.75% | 4.03% |
| criterion:actions/validate_scale.v1 | 21.375 | 21.997 | -2.91% | 0.438 | 0.585 | 7.87% | 1.58% |
| criterion:api/group_500_hosts_and_containers.v1 | 265131.021 | 263391.008 | +0.66% | 5195.924 | 5124.847 | 8.00% | 2.92% |
| criterion:api/group_500_hosts_cpu_utilization_fallback.v1 | 227730.974 | 152597.250 | +32.99% | 9937.905 | 2447.626 | 25.57% | 3.13% |
| criterion:api/group_500_hosts_realistic_metrics.v1 | 1496950.417 | 963775.802 | +35.62% | 40782.053 | 16847.318 | 11.89% | 28.83% |
| criterion:api/place_200_resources_600_addresses.v1 | 1679500.546 | 1675782.815 | +0.22% | 69477.385 | 34534.823 | 16.28% | 3.93% |
| criterion:archive/decode_512_logs.v1 | 347398.192 | 348815.753 | -0.41% | 8726.195 | 5059.317 | 13.53% | 1.53% |
| criterion:archive/encode_512_logs.v1 | 311250.172 | 318398.977 | -2.30% | 15139.340 | 34214.220 | 16.00% | 2.83% |
| criterion:archive/index_512_logs.v1 | 83578.791 | 83427.692 | +0.18% | 1923.956 | 1090.666 | 14.05% | 2.07% |
| criterion:archiver/plan_closed_windows.v1 | 428.869 | 420.225 | +2.02% | 14.382 | 8.085 | 21.34% | 2.38% |
| criterion:cli/parse_node_entry.v1 | 107.701 | 105.262 | +2.26% | 5.515 | 1.428 | 22.86% | 3.87% |
| criterion:convert/normalize_1000_logs.v1 | 1158822.941 | 1143530.873 | +1.32% | 28738.693 | 16971.715 | 15.94% | 3.95% |
| criterion:convert/normalize_1000_spans.v1 | 954472.072 | 950108.987 | +0.46% | 29076.714 | 19127.484 | 17.50% | 1.37% |
| criterion:convert/normalize_256_custom_events.v1 | 57776.128 | 56066.443 | +2.96% | 2609.337 | 779.414 | 22.81% | 5.43% |
| criterion:convert/normalize_256_metric_points.v1 | 304690.063 | 302946.598 | +0.57% | 10482.730 | 5958.556 | 16.03% | 4.12% |
| criterion:domain/classify_common_attribute.v1 | 353.885 | 344.797 | +2.57% | 11.260 | 4.641 | 11.86% | 0.59% |
| criterion:domain/classify_header.v1 | 49.472 | 47.638 | +3.71% | 1.740 | 1.129 | 11.70% | 1.97% |
| criterion:domain/derive_scoped_service_id.v1 | 199.281 | 193.516 | +2.89% | 5.628 | 2.472 | 14.96% | 4.37% |
| criterion:domain/mask_bearer_plain_value.v1 | 51.993 | 50.262 | +3.33% | 2.308 | 1.014 | 13.90% | 3.18% |
| criterion:domain/mask_bearer_secret_value.v1 | 72.625 | 69.561 | +4.22% | 2.420 | 1.727 | 17.00% | 4.68% |
| criterion:domain/redact_url_attribute.v1 | 120.788 | 118.808 | +1.64% | 4.697 | 1.707 | 15.46% | 4.97% |
| criterion:domain/sanitize_common_string_attribute.v1 | 33.606 | 32.790 | +2.43% | 1.119 | 0.349 | 17.05% | 2.52% |
| criterion:domain/sanitize_common_string_attribute_in_place.v1 | 12.601 | 12.400 | +1.60% | 0.485 | 0.412 | 11.18% | 2.87% |
| criterion:engine/apply_batch_2000_spans.v1 | 758957.475 | 624481.178 | +17.72% | 30097.815 | 8840.490 | 17.06% | 2.56% |
| criterion:engine/apply_batch_json_2000_spans.v1 | 2275559.034 | 2160761.475 | +5.04% | 71759.151 | 37404.677 | 14.04% | 1.69% |
| criterion:engine/build_server_geometry_100_nodes.v1 | 869025.407 | 813741.313 | +6.36% | 23571.701 | 15012.416 | 28.81% | 1.67% |
| criterion:engine/search_visible_state_1000_nodes.v1 | 23685.055 | 22609.416 | +4.54% | 770.033 | 347.809 | 13.43% | 7.58% |
| criterion:engine/seek_and_build_visible_state.v1 | 23198.935 | 23220.140 | -0.09% | 848.358 | 386.363 | 18.70% | 2.55% |
| criterion:executor/evaluate_and_record_verification.v1 | 1303.449 | 1256.435 | +3.61% | 35.185 | 18.485 | 15.60% | 7.65% |
| criterion:ingest/sanitize_1000_logs_with_attributes.v1 | 575954.536 | 569101.171 | +1.19% | 13140.883 | 13068.133 | 5.07% | 1.49% |
| criterion:ingest/sanitize_1000_metric_points_with_attributes.v1 | 865767.390 | 858169.622 | +0.88% | 18454.563 | 16353.872 | 4.29% | 0.74% |
| criterion:ingest/sanitize_1000_traces.v1 | 43117.983 | 41644.174 | +3.42% | 7652.687 | 9330.764 | 21.53% | 11.16% |
| criterion:ingest/sanitize_1000_traces_with_attributes.v1 | 730089.034 | 701545.467 | +3.91% | 30152.063 | 17388.173 | 17.02% | 2.38% |
| criterion:processor/check_dead_letter_payload_bound.v1 | 0.566 | 0.480 | +15.18% | 0.191 | 0.124 | 11.18% | 32.24% |
| criterion:processor/drain_128_ready_records.v1 | 203.415 | 197.856 | +2.73% | 23.031 | 20.958 | 5.07% | 2.96% |
| criterion:protocol/decode_256_events.v1 | 27384.397 | 26608.420 | +2.83% | 489.119 | 532.903 | 6.45% | 1.23% |
| criterion:protocol/encode_256_events.v1 | 5654.499 | 5429.938 | +3.97% | 93.158 | 71.996 | 3.59% | 3.43% |
| criterion:runtime/backoff_schedule.v1 | 14.330 | 14.287 | +0.30% | 0.223 | 0.149 | 4.51% | 4.51% |
| criterion:storage/parse_clickhouse_ttl.v1 | 155.705 | 156.697 | -0.64% | 2.142 | 2.194 | 5.13% | 6.14% |
| criterion:web_engine/construct_from_config_json.v1 | 215.065 | 225.496 | -4.85% | 2.778 | 5.600 | 5.98% | 3.75% |

Generated from stored Criterion summaries. Variation is represented by Criterion's estimated standard deviation; no significance claim is made by this report.
