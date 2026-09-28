"use client";

import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { api, scopedPath, unwrap } from "@/lib/api";
import { formatClock, parseNs } from "@/lib/time";
import type { PanelScope } from "./BottomPanel";

export function EventsPanel({ scope, onTrace }: { scope: PanelScope; onTrace: (id: string) => void }) {
  const query = useQuery({
    queryKey: ["events", scope.projectId, scope.environmentId, scope.from.toString(), scope.to.toString()],
    queryFn: async () =>
      unwrap(
        await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/events", {
          params: {
            path: scopedPath(scope.projectId, scope.environmentId),
            query: { from: scope.from.toString(), to: scope.to.toString(), limit: 500 },
          },
        }),
      ),
    placeholderData: keepPreviousData,
    refetchInterval: scope.live ? 3000 : false,
  });
  const events = (query.data?.events ?? []).filter((e) => !scope.serviceName || e.service_name === scope.serviceName);

  if (query.error) {
    return (
      <p role="alert" className="p-3 text-xs text-error">
        Failed to load events: {query.error.message}
      </p>
    );
  }
  if (events.length === 0 && !query.isPending) return <p className="p-3 text-xs text-muted">No events in this window.</p>;

  return (
    <div className="h-full overflow-auto">
      <table className="w-full text-left text-xs">
        <thead className="sticky top-0 bg-surface text-muted">
          <tr>
            <th scope="col" className="px-3 py-1">Time</th>
            <th scope="col" className="px-3 py-1">Type</th>
            <th scope="col" className="px-3 py-1">Service</th>
            <th scope="col" className="px-3 py-1">Attributes</th>
            <th scope="col" className="px-3 py-1">Trace</th>
          </tr>
        </thead>
        <tbody className="font-mono">
          {events.slice(0, 500).map((e) => {
            const t = parseNs(e.time_ns);
            const attrs = Object.entries(e.attributes)
              .map(([k, v]) => `${k}=${v}`)
              .join(" ");
            return (
              <tr key={e.id} className="border-t border-border">
                <td className="px-3 py-0.5 text-muted">{t === null ? "" : formatClock(t)}</td>
                <td className="px-3 py-0.5 text-accent">{e.event_type}</td>
                <td className="px-3 py-0.5">{e.service_name}</td>
                <td className="max-w-md truncate px-3 py-0.5 text-muted" title={attrs}>
                  {attrs}
                </td>
                <td className="px-3 py-0.5">
                  {e.trace_id ? (
                    <button type="button" className="text-accent hover:underline" onClick={() => e.trace_id && onTrace(e.trace_id)}>
                      open
                    </button>
                  ) : null}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
