"use client";

import { useQuery } from "@tanstack/react-query";
import { api, scopedPath, unwrap } from "@/lib/api";
import { nowNs } from "@/lib/time";
import { hasMetricOverviewData, metricOverviewTiles } from "./metric-overview";

interface Props {
  projectId: string;
  environmentId: string;
  name: string;
  title: string;
  dependency: boolean;
  cursor: bigint;
  live: boolean;
}

export function MetricOverviewCard({ projectId, environmentId, name, title, dependency, cursor, live }: Props) {
  const selectedTime = live ? "live" : cursor.toString();
  const overview = useQuery({
    queryKey: ["metric-overview", projectId, environmentId, name, selectedTime],
    queryFn: async () => unwrap(await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/metrics/overview", {
      params: { path: scopedPath(projectId, environmentId), query: { name, at: (live ? nowNs() : cursor).toString() } },
    })),
    refetchInterval: live ? 10_000 : false,
  });

  return (
    <section aria-label={title} className="rounded-xl border border-border bg-surface-2/50 p-3">
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-xs font-semibold text-text">{title}</h3>
        <span className="text-[10px] text-muted">Last 5 min to cursor</span>
      </div>
      {overview.isPending ? <div role="status" className="mt-3 h-16 animate-pulse rounded-lg bg-surface-3" aria-label={`Loading ${title}`} /> : null}
      {overview.isError ? <p role="status" className="mt-2 text-xs text-warn">Metrics are unavailable.</p> : null}
      {overview.data && !hasMetricOverviewData(overview.data, dependency) ? (
        <p className="mt-2 text-xs text-muted">No {dependency ? "dependency" : "application"} metrics in this interval.</p>
      ) : null}
      {overview.data && hasMetricOverviewData(overview.data, dependency) ? (
        <dl className="mt-3 grid grid-cols-2 gap-2">
          {metricOverviewTiles(overview.data, dependency).map((tile) => (
            <div key={tile.label} className="rounded-lg border border-border bg-surface px-2.5 py-2">
              <dt className="text-[10px] text-muted">{tile.label}</dt>
              <dd className="mt-1 font-mono text-sm font-semibold text-text">{tile.value}</dd>
            </div>
          ))}
        </dl>
      ) : null}
    </section>
  );
}
