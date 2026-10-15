import type { Schemas } from "@/lib/api";

type Overview = Schemas["MetricOverviewResponse"];

export function metricOverviewTiles(data: Overview, dependency: boolean) {
  if (dependency) {
    return [
      { label: "Connections", value: data.postgres_backends == null ? "—" : String(Math.round(data.postgres_backends)) },
      { label: "Commits/s", value: data.postgres_commits_per_second == null ? "—" : data.postgres_commits_per_second.toFixed(1) },
      { label: "Rollbacks/s", value: data.postgres_rollbacks_per_second == null ? "—" : data.postgres_rollbacks_per_second.toFixed(1) },
      { label: "Deadlocks/s", value: data.postgres_deadlocks_per_second == null ? "—" : data.postgres_deadlocks_per_second.toFixed(2) },
    ];
  }
  return [
    { label: "HTTP p95 ≤", value: data.http_p95_ms == null ? "—" : `${data.http_p95_ms.toFixed(0)} ms` },
    { label: "HTTP samples", value: data.http_samples.toLocaleString() },
    { label: "Process RSS", value: data.process_rss_bytes == null ? "—" : `${(data.process_rss_bytes / 1_048_576).toFixed(1)} MiB` },
    { label: "CPU / core", value: data.process_cpu_pct_of_core == null ? "—" : `${data.process_cpu_pct_of_core.toFixed(1)}%` },
  ];
}

export function hasMetricOverviewData(data: Overview, dependency: boolean): boolean {
  return dependency ? data.postgres_backends != null || data.postgres_commits_per_second != null
    : data.http_samples > 0 || data.process_rss_bytes != null;
}
