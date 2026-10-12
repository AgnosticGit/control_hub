# Performance comparison: baseline:initial → 20261012T031539183Z_windows

Compatibility key: `windows-x86_64-msvc|rustc 1.98.1 (48a229cea 2026-09-01)|bench|default|lock-7916af49e5073df7c73aa2a57829cfd42d37593eabea06d415a182622c6146c5|workloads-v1`

| Workload | Before (ns) | After (ns) | Change (lower is better) | Before std dev (ns) | After std dev (ns) |
|---|---:|---:|---:|---:|---:|
| archive.encode-logs-512.v1 | 352957.372 | 333369.538 | +5.55% | 50952.397 | 27117.697 |
| engine.apply-batch-2000.v1 | 854432.446 | 890066.322 | -4.17% | 70711.832 | 150017.282 |
| ingest.sanitize-traces-1000.v1 | 947803.270 | 928726.601 | +2.01% | 199530.758 | 167819.253 |

Generated from stored Criterion summaries. Variation is represented by Criterion's estimated standard deviation; no significance claim is made by this report.
