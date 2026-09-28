"use client";

import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef, useState } from "react";
import { api, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { formatClock, parseNs } from "@/lib/time";
import type { PanelScope } from "./BottomPanel";
import { useDebounced } from "./useDebounced";

type Log = Schemas["LogDto"];

const SEVERITIES = [
  { label: "All", value: 0 },
  { label: "Warn+", value: 13 },
  { label: "Error+", value: 17 },
];

function severityClass(n: number) {
  if (n >= 17) return "text-error";
  if (n >= 13) return "text-warn";
  return "text-muted";
}

export function LogsPanel({ scope, onTrace }: { scope: PanelScope; onTrace: (id: string) => void }) {
  const [minSeverity, setMinSeverity] = useState(0);
  const [text, setText] = useState("");
  const q = useDebounced(text.trim(), 300);

  const query = useQuery({
    queryKey: ["logs", scope.projectId, scope.environmentId, scope.from.toString(), scope.to.toString(), scope.serviceId, minSeverity, q],
    queryFn: async () =>
      unwrap(
        await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/logs", {
          params: {
            path: scopedPath(scope.projectId, scope.environmentId),
            query: {
              from: scope.from.toString(),
              to: scope.to.toString(),
              limit: 1000,
              ...(scope.serviceId ? { service_id: scope.serviceId } : {}),
              ...(minSeverity > 0 ? { min_severity: minSeverity } : {}),
              ...(q ? { q } : {}),
            },
          },
        }),
      ),
    placeholderData: keepPreviousData,
    refetchInterval: scope.live ? 3000 : false,
  });

  const logs: Log[] = query.data?.logs ?? [];
  const parentRef = useRef<HTMLDivElement | null>(null);
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual returns non-memoizable functions by design
  const virtualizer = useVirtualizer({
    count: logs.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 24,
    overscan: 20,
  });

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-xs">
        <input
          type="search"
          placeholder="Search log text…"
          aria-label="Search log text"
          value={text}
          onChange={(e) => setText(e.target.value)}
          className="w-64 rounded border border-border bg-bg px-2 py-1"
        />
        <div role="group" aria-label="Minimum severity" className="flex gap-1">
          {SEVERITIES.map((s) => (
            <button
              key={s.value}
              type="button"
              aria-pressed={minSeverity === s.value}
              onClick={() => setMinSeverity(s.value)}
              className={`rounded border px-2 py-0.5 ${minSeverity === s.value ? "border-accent text-accent" : "border-border text-muted"}`}
            >
              {s.label}
            </button>
          ))}
        </div>
        <span className="ml-auto text-muted">
          {query.isFetching ? "Loading… " : ""}
          {logs.length} logs{query.data?.completeness.truncated ? " (truncated — narrow the filter)" : ""}
        </span>
      </div>
      {query.error ? (
        <p role="alert" className="p-3 text-xs text-error">
          Failed to load logs: {query.error.message}
        </p>
      ) : logs.length === 0 && !query.isPending ? (
        <p className="p-3 text-xs text-muted">No logs in this window.</p>
      ) : (
        <div ref={parentRef} className="min-h-0 flex-1 overflow-auto font-mono text-xs" aria-label="Log records">
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((row) => {
              const l = logs[row.index];
              if (!l) return null;
              const t = parseNs(l.time_ns);
              return (
                <div
                  key={l.id}
                  className="absolute left-0 flex w-full items-center gap-3 whitespace-nowrap px-3 hover:bg-surface-2"
                  style={{ top: row.start, height: row.size }}
                >
                  <span className="text-muted tabular-nums">{t === null ? "?" : formatClock(t)}</span>
                  <span className={`w-12 ${severityClass(l.severity_number)}`}>{l.severity_text || l.severity_number}</span>
                  <span className="w-28 truncate text-accent-2">{l.service_name}</span>
                  <span className="min-w-0 flex-1 truncate" title={l.body}>
                    {l.body}
                  </span>
                  {l.trace_id ? (
                    <button type="button" className="text-accent hover:underline" onClick={() => l.trace_id && onTrace(l.trace_id)}>
                      trace
                    </button>
                  ) : null}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
