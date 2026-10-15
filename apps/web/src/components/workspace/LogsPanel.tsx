"use client";

import { keepPreviousData, useInfiniteQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { formatClock, parseNs } from "@/lib/time";
import type { PanelScope } from "./SignalsPanel";
import type { WorkspaceActions } from "./useWorkspaceEngine";
import { useDebounced } from "./useDebounced";
import {
  SOURCE_CHOICES,
  combinedSource,
  flattenPages,
  hasTemplate,
  logQuery,
  sourceBadge,
  type LogFilters,
  type LogSourceChoice,
} from "./logs-query";

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

export function LogsPanel({ scope, actions, onTrace, onSelectRecord, expandedId, onExpandedIdChange }: {
  scope: PanelScope;
  actions: WorkspaceActions;
  onTrace: (id: string) => void;
  onSelectRecord: (serviceName: string, time: bigint) => void;
  expandedId: string | null;
  onExpandedIdChange: (id: string | null) => void;
}) {
  const [minSeverity, setMinSeverity] = useState(0);
  const [text, setText] = useState("");
  const [source, setSource] = useState<LogSourceChoice>("auto");
  const [like, setLike] = useState<{ id: string; body: string } | null>(null);
  const [liveLogs, setLiveLogs] = useState<Log[]>([]);
  const q = useDebounced(text.trim(), 300);

  const filters: LogFilters = {
    from: scope.from,
    to: scope.to,
    serviceId: scope.serviceId,
    minSeverity,
    q,
    source,
    templateId: like?.id ?? null,
  };

  const query = useInfiniteQuery({
    queryKey: [
      "logs",
      scope.projectId,
      scope.environmentId,
      scope.from.toString(),
      scope.to.toString(),
      scope.serviceId,
      minSeverity,
      q,
      source,
      like?.id ?? null,
    ],
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam }) =>
      unwrap(
        await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/logs", {
          params: { path: scopedPath(scope.projectId, scope.environmentId), query: logQuery(filters, pageParam) },
        }),
      ),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
    placeholderData: keepPreviousData,
    refetchInterval: false,
  });

  const pages = useMemo(() => query.data?.pages ?? [], [query.data?.pages]);
  useEffect(() => {
    if (!scope.live) return;
    return actions.onLiveSignals((signals) => {
      const incoming = signals.filter((signal) => signal.kind === "log").map((signal) => signal.record as Log);
      if (incoming.length) setLiveLogs((current) => [...current, ...incoming].slice(-2000));
    });
  }, [actions, scope.live]);
  const logs: Log[] = useMemo(() => {
    const base = flattenPages(pages).filter((log) => {
      const time = BigInt(log.time_ns);
      return time >= scope.displayFrom && time < scope.displayTo;
    });
    if (!scope.live || source === "archive") return base;
    const inRange = liveLogs.filter((log) => {
      const time = BigInt(log.time_ns);
      return time >= scope.displayFrom && time < scope.displayTo
        && (!scope.serviceId || log.service_id === scope.serviceId)
        && log.severity_number >= minSeverity
        && (!q || log.body.toLocaleLowerCase().includes(q.toLocaleLowerCase()))
        && (!like || log.template_id === like.id);
    });
    const merged = new Map(base.map((log) => [log.id, log]));
    for (const log of inRange) merged.set(log.id, log);
    return [...merged.values()].sort((a, b) => (BigInt(a.time_ns) > BigInt(b.time_ns) ? -1 : 1));
  }, [pages, scope.live, scope.displayFrom, scope.displayTo, scope.serviceId, source, liveLogs, minSeverity, q, like]);
  const badge = sourceBadge(combinedSource(pages));
  const parentRef = useRef<HTMLDivElement | null>(null);
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual returns non-memoizable functions by design
  const virtualizer = useVirtualizer({
    count: logs.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 88,
    overscan: 20,
  });

  return (
    <div className="flex h-full flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2 text-xs">
        <input
          type="search"
          placeholder="Search log text…"
          aria-label="Search log text"
          value={text}
          onChange={(e) => setText(e.target.value)}
          className="input h-7 min-w-32 flex-1 text-xs @2xl:max-w-64"
        />
        <div role="group" aria-label="Minimum severity" className="seg">
          {SEVERITIES.map((s) => (
            <button
              key={s.value}
              type="button"
              aria-pressed={minSeverity === s.value}
              onClick={() => setMinSeverity(s.value)}
              className="seg-item py-0.5"
            >
              {s.label}
            </button>
          ))}
        </div>
        <div role="group" aria-label="Log source" className="seg">
          {SOURCE_CHOICES.map((s) => (
            <button
              key={s.value}
              type="button"
              title={s.title}
              aria-pressed={source === s.value}
              onClick={() => setSource(s.value)}
              className="seg-item py-0.5"
            >
              {s.label}
            </button>
          ))}
        </div>
        {badge ? (
          <span title={badge.title} className="rounded-md border border-violet/40 bg-violet/10 px-1.5 py-0.5 text-[11px] text-violet">
            {badge.text}
          </span>
        ) : null}
        <span className="ml-auto text-muted">
          {query.isFetching ? "Loading… " : ""}
          {logs.length} {logs.length === 1 ? "log" : "logs"}
        </span>
      </div>
      {like ? (
        <div className="flex items-center gap-2 border-b border-border px-3 py-1 text-xs">
          <span className="shrink-0 text-muted">Lines like</span>
          <span className="min-w-0 flex-1 truncate font-mono" title={like.body}>
            {like.body}
          </span>
          <button type="button" className="btn btn-sm" onClick={() => setLike(null)}>
            Show all lines
          </button>
        </div>
      ) : null}
      {query.error ? (
        <p role="alert" className="p-3 text-xs text-error">
          Failed to load logs: {query.error.message}
        </p>
      ) : logs.length === 0 && !query.isPending && !query.hasNextPage ? (
        <div className="m-3 rounded-xl border border-border bg-surface-2 p-4 text-sm">
          <p className="font-medium">{q || minSeverity || like ? "No logs match these filters" : "No logs in this time range"}</p>
          <p className="mt-1 text-xs text-muted">Try a broader time range, clear the search or lower the severity filter.</p>
        </div>
      ) : (
        <div ref={parentRef} className="min-h-0 flex-1 overflow-auto px-2 py-1" aria-label="Log records">
          <div role="list" aria-label="Log entries" style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {virtualizer.getVirtualItems().map((row) => {
              const l = logs[row.index];
              if (!l) return null;
              const t = parseNs(l.time_ns);
              const expanded = expandedId === l.id;
              const traceId = l.trace_id;
              return (
                <div
                  key={l.id}
                  role="listitem"
                  ref={virtualizer.measureElement}
                  data-index={row.index}
                  className="absolute left-0 w-full px-1 pb-2"
                  style={{ top: row.start }}
                >
                  <article className={`rounded-xl border bg-surface px-3 py-2 transition-colors ${expanded ? "border-accent/50 bg-surface-2" : "border-border hover:border-border-strong hover:bg-surface-2"}`}>
                    <div className="flex items-start gap-2">
                      <button
                        type="button"
                        className="min-w-0 flex-1 text-left"
                        aria-expanded={expanded}
                        aria-label={`${l.severity_text || `Severity ${l.severity_number}`} log from ${l.service_name} at ${t === null ? "unknown time" : formatClock(t)}`}
                        onClick={() => {
                          onExpandedIdChange(expanded ? null : l.id);
                          if (t !== null) onSelectRecord(l.service_name, t);
                        }}
                      >
                        <span className="flex min-w-0 items-center gap-2">
                          <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted">{t === null ? "?" : formatClock(t)}</span>
                          <span className={`rounded-md border border-current/25 px-1.5 py-0.5 text-[10px] font-semibold uppercase ${severityClass(l.severity_number)}`}>
                            {l.severity_text || `Level ${l.severity_number}`}
                          </span>
                          <span className="min-w-0 flex-1 truncate text-[11px] text-accent-2">{l.service_name}</span>
                          <span className="shrink-0 text-[10px] text-muted">{expanded ? "Hide details" : "Details"}</span>
                        </span>
                        <span className={`mt-1 block text-xs leading-5 text-text ${expanded ? "whitespace-pre-wrap break-words" : "line-clamp-2 break-words"}`}>{l.body}</span>
                      </button>
                      <div className="flex shrink-0 flex-col gap-1">
                        {hasTemplate(l) && like?.id !== l.template_id ? (
                          <button type="button" aria-label="Show lines like this" className="btn btn-sm px-2" onClick={() => setLike({ id: l.template_id, body: l.body })} title="Show records with this message pattern">
                            Similar
                          </button>
                        ) : null}
                        {traceId ? <button type="button" className="btn btn-sm px-2 text-accent" onClick={() => onTrace(traceId)}>Open trace</button> : null}
                      </div>
                    </div>
                    {expanded ? (
                      <div className="mt-2 border-t border-border pt-2 text-xs">
                        <p className="mb-1 text-[10px] font-semibold tracking-wide text-muted uppercase">Log details</p>
                        <p className="whitespace-pre-wrap break-words">{l.body}</p>
                        {Object.keys(l.attributes).length > 0 ? (
                          <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1">
                            {Object.entries(l.attributes).sort(([a], [b]) => a.localeCompare(b)).map(([key, value]) => (
                              <div key={key} className="contents"><dt className="text-muted">{key}</dt><dd className="min-w-0 break-all">{value}</dd></div>
                            ))}
                          </dl>
                        ) : <p className="mt-2 text-muted">No additional attributes.</p>}
                        {traceId || l.span_id ? <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 font-mono text-[10px]">
                          {traceId ? <><dt className="text-muted">Trace ID</dt><dd className="break-all">{traceId}</dd></> : null}
                          {l.span_id ? <><dt className="text-muted">Span ID</dt><dd className="break-all">{l.span_id}</dd></> : null}
                        </dl> : null}
                      </div>
                    ) : null}
                  </article>
                </div>
              );
            })}
          </div>
        </div>
      )}
      {query.hasNextPage ? (
        <div className="flex items-center gap-2 border-t border-border px-3 py-1.5 text-xs">
          <span className="text-muted">More logs exist in this window.</span>
          <button type="button" className="btn btn-sm ml-auto" disabled={query.isFetchingNextPage} onClick={() => void query.fetchNextPage()}>
            {query.isFetchingNextPage ? "Loading…" : "Load older"}
          </button>
        </div>
      ) : null}
    </div>
  );
}
