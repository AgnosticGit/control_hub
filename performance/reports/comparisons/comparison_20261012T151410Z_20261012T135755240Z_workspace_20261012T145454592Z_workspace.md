# Performance comparison: 20261012T135755240Z_workspace → 20261012T145454592Z_workspace

Compatibility key: `workspace-criterion-v1|x86_64-pc-windows-msvc|rustc 1.98.1 (48a229cea 2026-09-01)|bench|defaults|lock-367c195a541a6c4c46910a0547842fd39d9947c8f8b0bbf3842b060c74a25a41`

| Workload | Before mean (ns) | After mean (ns) | Change | Before within-run SD | After within-run SD | Before run range | After run range |
|---|---:|---:|---:|---:|---:|---:|---:|
| criterion:actions/evaluate_policy.v1 | 252.100 | 246.433 | +2.25% | 4.769 | 4.829 | 8.15% | 4.14% |
| criterion:actions/validate_scale.v1 | 21.565 | 21.249 | +1.46% | 0.345 | 0.322 | 1.51% | 4.61% |
| criterion:api/group_500_hosts_and_containers.v1 | 262988.145 | 271255.266 | -3.14% | 5338.743 | 5012.638 | 5.71% | 3.22% |
| criterion:api/group_500_hosts_cpu_utilization_fallback.v1 | 153735.837 | 149131.411 | +3.00% | 2636.433 | 2658.190 | 7.48% | 6.89% |
| criterion:api/group_500_hosts_realistic_metrics.v1 | 868433.004 | 859590.784 | +1.02% | 19304.350 | 26414.176 | 3.99% | 1.09% |
| criterion:api/place_200_resources_600_addresses.v1 | 1661259.408 | 1601383.950 | +3.60% | 32560.452 | 29144.825 | 4.24% | 2.89% |
| criterion:archive/decode_512_logs.v1 | 360759.216 | 356884.943 | +1.07% | 5448.784 | 7133.980 | 0.76% | 4.13% |
| criterion:archive/encode_512_logs.v1 | 320782.755 | 309382.020 | +3.55% | 47305.413 | 31859.292 | 11.33% | 5.55% |
| criterion:archive/index_512_logs.v1 | 84310.376 | 83794.010 | +0.61% | 1350.342 | 1398.192 | 4.45% | 2.95% |
| criterion:archiver/plan_closed_windows.v1 | 429.306 | 421.925 | +1.72% | 18.458 | 8.081 | 3.68% | 4.26% |
| criterion:cli/parse_node_entry.v1 | 105.095 | 106.559 | -1.39% | 1.487 | 1.854 | 2.13% | 1.78% |
| criterion:convert/normalize_1000_logs.v1 | 1162258.904 | 1150574.667 | +1.01% | 23986.155 | 24296.407 | 4.01% | 4.31% |
| criterion:convert/normalize_1000_spans.v1 | 965648.255 | 961332.721 | +0.45% | 16780.367 | 22028.731 | 2.60% | 3.10% |
| criterion:convert/normalize_256_custom_events.v1 | 55117.384 | 55021.469 | +0.17% | 991.317 | 774.019 | 1.19% | 1.19% |
| criterion:convert/normalize_256_metric_points.v1 | 305206.574 | 304430.577 | +0.25% | 4979.108 | 4511.295 | 1.68% | 0.62% |
| criterion:domain/classify_common_attribute.v1 | 349.788 | 348.828 | +0.27% | 4.621 | 5.422 | 3.19% | 2.74% |
| criterion:domain/classify_header.v1 | 49.625 | 49.232 | +0.79% | 1.075 | 1.166 | 5.58% | 2.01% |
| criterion:domain/derive_scoped_service_id.v1 | 195.740 | 193.286 | +1.25% | 4.547 | 2.615 | 5.31% | 3.62% |
| criterion:domain/mask_bearer_plain_value.v1 | 51.534 | 51.348 | +0.36% | 1.136 | 0.839 | 1.97% | 3.41% |
| criterion:domain/mask_bearer_secret_value.v1 | 69.845 | 70.482 | -0.91% | 1.320 | 1.382 | 2.79% | 3.56% |
| criterion:domain/redact_url_attribute.v1 | 120.268 | 119.689 | +0.48% | 1.903 | 1.463 | 5.15% | 2.58% |
| criterion:domain/sanitize_common_string_attribute.v1 | 33.209 | 33.080 | +0.39% | 0.538 | 0.475 | 3.97% | 1.39% |
| criterion:domain/sanitize_common_string_attribute_in_place.v1 | 12.616 | 12.449 | +1.33% | 0.389 | 0.464 | 5.36% | 0.84% |
| criterion:engine/apply_batch_2000_spans.v1 | 671759.114 | 635534.971 | +5.39% | 53198.004 | 12799.993 | 13.95% | 0.88% |
| criterion:engine/apply_batch_json_2000_spans.v1 | 2330158.542 | 2150820.574 | +7.70% | 177188.398 | 34687.380 | 18.48% | 2.40% |
| criterion:engine/build_server_geometry_100_nodes.v1 | 826765.650 | 820886.110 | +0.71% | 17688.664 | 10463.502 | 3.83% | 3.19% |
| criterion:engine/search_visible_state_1000_nodes.v1 | 25198.063 | 22068.453 | +12.42% | 2759.330 | 362.494 | 39.87% | 2.06% |
| criterion:engine/seek_and_build_visible_state.v1 | 24513.732 | 22602.237 | +7.80% | 1902.269 | 410.871 | 24.93% | 1.40% |
| criterion:executor/evaluate_and_record_verification.v1 | 1279.521 | 1255.999 | +1.84% | 22.348 | 23.407 | 2.26% | 2.88% |
| criterion:ingest/sanitize_1000_logs_with_attributes.v1 | 667106.823 | 665673.395 | +0.21% | 15721.875 | 23828.553 | 0.28% | 1.10% |
| criterion:ingest/sanitize_1000_metric_points_with_attributes.v1 | 919386.023 | 921443.852 | -0.22% | 16911.037 | 12871.706 | 1.16% | 3.60% |
| criterion:ingest/sanitize_1000_traces.v1 | 42421.700 | 41223.802 | +2.82% | 4526.714 | 4624.379 | 5.99% | 3.17% |
| criterion:ingest/sanitize_1000_traces_with_attributes.v1 | 786994.693 | 789598.132 | -0.33% | 18458.168 | 19442.486 | 2.19% | 2.65% |
| criterion:processor/check_dead_letter_payload_bound.v1 | 0.442 | 0.448 | -1.46% | 0.107 | 0.086 | 31.86% | 44.44% |
| criterion:processor/drain_128_ready_records.v1 | 198.539 | 202.601 | -2.05% | 23.231 | 24.889 | 1.49% | 3.47% |
| criterion:protocol/decode_256_events.v1 | 34457.851 | 26680.305 | +22.57% | 2419.393 | 874.436 | 61.03% | 5.10% |
| criterion:protocol/encode_256_events.v1 | 5664.092 | 5557.585 | +1.88% | 207.540 | 106.066 | 5.43% | 0.33% |
| criterion:runtime/backoff_schedule.v1 | 17.447 | 14.266 | +18.23% | 0.715 | 0.192 | 52.58% | 0.90% |
| criterion:storage/parse_clickhouse_ttl.v1 | 191.262 | 153.039 | +19.98% | 10.735 | 2.403 | 55.33% | 2.70% |
| criterion:web_engine/construct_from_config_json.v1 | 275.888 | 215.957 | +21.72% | 19.231 | 2.670 | 60.86% | 3.41% |

Generated from stored Criterion summaries. Variation is represented by Criterion's estimated standard deviation; no significance claim is made by this report.
