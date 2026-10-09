"use client";

import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { formatClock, formatDurationNs, parseNs } from "@/lib/time";
import type { PanelScope } from "./SignalsPanel";
import type { WorkspaceActions } from "./useWorkspaceEngine";

type Span = Schemas["SpanDto"];

export function TracesPanel({
  scope,
  actions,
  traceId,
  onTrace,
}: {
  scope: PanelScope;
  actions: WorkspaceActions;
  traceId: string | null;
  onTrace: (id: string | null) => void;
}) {
  const [onlyErrors, setOnlyErrors] = useState(false);
  const [liveSpans, setLiveSpans] = useState<Map<string, Span>>(() => new Map());
  const list = useQuery({
    queryKey: ["traces", scope.projectId, scope.environmentId, scope.from.toString(), scope.to.toString(), scope.serviceId, onlyErrors],
    queryFn: async () =>
      unwrap(
        await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/traces", {
          params: {
            path: scopedPath(scope.projectId, scope.environmentId),
            query: {
              from: scope.from.toString(),
              to: scope.to.toString(),
              limit: 100,
              only_errors: onlyErrors,
              ...(scope.serviceId ? { service_id: scope.serviceId } : {}),
            },
          },
        }),
      ),
    placeholderData: keepPreviousData,
    refetchInterval: false,
  });
  useEffect(() => {
    if (!scope.live) return;
    return actions.onLiveSignals((signals) => {
      const incoming = signals.filter((signal) => signal.kind === "span").map((signal) => signal.record as Span);
      if (!incoming.length) return;
      setLiveSpans((current) => {
        const next = new Map(current);
        for (const span of incoming) next.set(span.span_id, span);
        while (next.size > 2000) {
          const oldest = next.keys().next().value;
          if (oldest === undefined) break;
          next.delete(oldest);
        }
        return next;
      });
    });
  }, [actions, scope.live]);
  const traces = useMemo(() => {
    const stored = (list.data?.traces ?? []).filter((trace) => {
      const start = BigInt(trace.start_ns);
      return start >= scope.displayFrom && start < scope.displayTo;
    });
    if (!scope.live || liveSpans.size === 0) return stored;
    const known = new Set(stored.map((trace) => trace.trace_id));
    const groups = new Map<string, Span[]>();
    for (const span of liveSpans.values()) {
      const start = BigInt(span.start_ns);
      if (start < scope.displayFrom || start >= scope.displayTo || known.has(span.trace_id)
        || (scope.serviceId && span.service_id !== scope.serviceId)
        || (onlyErrors && span.status_code !== 2)) continue;
      const group = groups.get(span.trace_id) ?? [];
      group.push(span);
      groups.set(span.trace_id, group);
    }
    const liveSummaries = [...groups].map(([trace_id, spans]) => {
      const start = spans.reduce((v, span) => BigInt(span.start_ns) < v ? BigInt(span.start_ns) : v, BigInt(spans[0]?.start_ns ?? "0"));
      const end = spans.reduce((v, span) => BigInt(span.end_ns) > v ? BigInt(span.end_ns) : v, BigInt(spans[0]?.end_ns ?? "0"));
      const root = spans.find((span) => !span.parent_span_id) ?? spans.reduce((a, b) => BigInt(a.start_ns) <= BigInt(b.start_ns) ? a : b);
      const services = [...new Set(spans.map((span) => span.service_name))].sort();
      return {
        trace_id,
        start_ns: start.toString(),
        end_ns: end.toString(),
        duration_ms: Number(end - start) / 1e6,
        span_count: spans.length,
        error_count: spans.filter((span) => span.status_code === 2).length,
        root_name: root.name,
        root_service: root.service_name,
        services,
      };
    });
    return [...stored, ...liveSummaries].sort((a, b) => BigInt(a.start_ns) > BigInt(b.start_ns) ? -1 : 1);
  }, [list.data?.traces, liveSpans, scope.live, scope.displayFrom, scope.displayTo, scope.serviceId, onlyErrors]);

  return (
    <div className="flex h-full flex-col @3xl:flex-row">
      <div className="flex max-h-[42%] min-h-28 shrink-0 flex-col border-b border-border @3xl:max-h-none @3xl:w-[42%] @3xl:min-w-72 @3xl:border-r @3xl:border-b-0">
        <div className="flex items-center gap-2 border-b border-border px-3 py-2 text-xs">
          <label className="toggle">
            <input type="checkbox" className="switch" checked={onlyErrors} onChange={(e) => setOnlyErrors(e.target.checked)} />
            Only errors
          </label>
          <span className="ml-auto text-muted">
            {list.isFetching ? "Loading… " : ""}
            {traces.length} traces
          </span>
        </div>
        {list.error ? (
          <p role="alert" className="p-3 text-xs text-error">
            Failed to load traces: {list.error.message}
          </p>
        ) : null}
        <ul className="min-h-0 flex-1 overflow-auto text-xs" aria-label="Traces">
          {traces.map((t) => {
            const start = parseNs(t.start_ns);
            return (
              <li key={t.trace_id}>
                <button
                  type="button"
                  onClick={() => onTrace(t.trace_id)}
                  aria-current={t.trace_id === traceId}
                  className={`flex w-full items-center gap-2 border-l-2 px-3 py-1.5 text-left transition-colors hover:bg-surface-2 ${t.trace_id === traceId ? "border-accent bg-accent/10" : "border-transparent"}`}
                >
                  <span className={`h-2 w-2 shrink-0 rounded-full ${t.error_count > 0 ? "bg-error" : "bg-ok"}`} aria-label={t.error_count > 0 ? "has errors" : "ok"} />
                  <span className="font-mono text-muted tabular-nums">{start === null ? "" : formatClock(start)}</span>
                  <span className="min-w-0 flex-1 truncate">
                    {t.root_service}: {t.root_name}
                  </span>
                  <span className="tabular-nums text-muted">{t.duration_ms.toFixed(1)} ms</span>
                  <span className="tabular-nums text-muted">{t.span_count}sp</span>
                </button>
              </li>
            );
          })}
          {traces.length === 0 && !list.isPending ? <li className="p-3 text-muted">No traces in this window.</li> : null}
        </ul>
      </div>
      <div className="min-h-0 min-w-0 flex-1 overflow-auto">
        {traceId ? <Waterfall scope={scope} traceId={traceId} /> : <p className="p-3 text-xs text-muted">Select a trace.</p>}
      </div>
    </div>
  );
}

interface Row {
  span: Span;
  depth: number;
}

function orderSpans(spans: Span[]): Row[] {
  const byId = new Map(spans.map((s) => [s.span_id, s]));
  const children = new Map<string, Span[]>();
  const roots: Span[] = [];
  for (const s of spans) {
    const p = s.parent_span_id;
    if (p && byId.has(p)) {
      const list = children.get(p) ?? [];
      list.push(s);
      children.set(p, list);
    } else {
      roots.push(s);
    }
  }
  const cmp = (a: Span, b: Span) => (BigInt(a.start_ns) < BigInt(b.start_ns) ? -1 : 1);
  const out: Row[] = [];
  const visit = (s: Span, depth: number) => {
    out.push({ span: s, depth });
    for (const c of (children.get(s.span_id) ?? []).sort(cmp)) visit(c, depth + 1);
  };
  for (const r of roots.sort(cmp)) visit(r, 0);
  return out;
}

function Waterfall({ scope, traceId }: { scope: PanelScope; traceId: string }) {
  const detail = useQuery({
    queryKey: ["trace", scope.projectId, scope.environmentId, traceId],
    queryFn: async () =>
      unwrap(
        await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/traces/{trace_id}", {
          params: { path: { ...scopedPath(scope.projectId, scope.environmentId), trace_id: traceId } },
        }),
      ),
  });
  const [selected, setSelected] = useState<string | null>(null);

  if (detail.isPending) return <p className="p-3 text-xs text-muted">Loading trace…</p>;
  if (detail.error) {
    return (
      <p role="alert" className="p-3 text-xs text-error">
        Failed to load trace: {detail.error.message}
      </p>
    );
  }
  const d = detail.data;
  const rows = orderSpans(d.spans);
  if (rows.length === 0) return <p className="p-3 text-xs text-muted">Trace has no spans in storage.</p>;
  let min = BigInt(rows[0]?.span.start_ns ?? "0");
  let max = BigInt(rows[0]?.span.end_ns ?? "0");
  for (const { span } of rows) {
    const s = BigInt(span.start_ns);
    const e = BigInt(span.end_ns);
    if (s < min) min = s;
    if (e > max) max = e;
  }
  const total = max - min > 0n ? max - min : 1n;
  const pct = (v: bigint) => Number(((v - min) * 10000n) / total) / 100;
  const sel = rows.find((r) => r.span.span_id === selected)?.span ?? null;

  return (
    <div className="text-xs">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border px-3 py-2">
        <span className="min-w-0 truncate font-mono text-muted" title={traceId}>
          {traceId}
        </span>
        <span className="text-muted">
          {rows.length} spans · {formatDurationNs(total)}
        </span>
        {d.missing_parent_span_ids.length > 0 ? (
          <span className="text-warn">{d.missing_parent_span_ids.length} parent span(s) missing — trace may be incomplete</span>
        ) : null}
        {d.truncated ? <span className="text-warn">truncated</span> : null}
      </div>
      <ul aria-label="Span waterfall">
        {rows.map(({ span, depth }) => {
          const left = pct(BigInt(span.start_ns));
          const width = Math.max(0.3, pct(BigInt(span.end_ns)) - left);
          const isErr = span.status_code === 2;
          return (
            <li key={span.span_id}>
              <button
                type="button"
                onClick={() => setSelected(span.span_id === selected ? null : span.span_id)}
                className={`flex w-full items-center gap-2 px-3 py-1 text-left hover:bg-surface-2 ${span.span_id === selected ? "bg-accent/10" : ""}`}
              >
                <span className="w-36 shrink-0 truncate @xl:w-64" style={{ paddingLeft: depth * 10 }} title={`${span.service_name} ${span.name}`}>
                  <span className="text-accent-2">{span.service_name}</span> {span.name}
                </span>
                <span className="relative h-2.5 flex-1 rounded-full bg-surface-3/60">
                  <span
                    className={`absolute top-0 h-2.5 rounded-full ${isErr ? "bg-error shadow-[0_0_8px_rgb(239_68_68/0.6)]" : "bg-gradient-to-r from-accent to-accent-2"}`}
                    style={{ left: `${left}%`, width: `${width}%` }}
                  />
                </span>
                <span className="w-16 shrink-0 text-right tabular-nums text-muted @xl:w-20">{span.duration_ms.toFixed(2)} ms</span>
              </button>
            </li>
          );
        })}
      </ul>
      {sel ? (
        <div className="m-3 rounded-lg border border-border bg-bg/50 p-3">
          <div className="font-medium">
            {sel.service_name} · {sel.name}
          </div>
          {sel.status_message ? <div className="mt-1 text-error">{sel.status_message}</div> : null}
          {sel.target_name ? <div className="mt-1 text-muted">Calls: {sel.target_name}</div> : null}
          <dl className="mt-2 grid grid-cols-[minmax(0,9rem)_1fr] @xl:grid-cols-[minmax(0,14rem)_1fr] gap-x-3 gap-y-0.5 font-mono">
            {Object.entries(sel.attributes).map(([k, v]) => (
              <div key={k} className="contents">
                <dt className="truncate text-muted">{k}</dt>
                <dd className="break-all">{v}</dd>
              </div>
            ))}
          </dl>
        </div>
      ) : null}
      {d.logs.length > 0 ? (
        <div className="m-3">
          <h3 className="mb-1 font-semibold text-muted">Correlated logs</h3>
          <ul className="font-mono">
            {d.logs.map((l) => {
              const t = parseNs(l.time_ns);
              return (
                <li key={l.id} className="flex gap-2">
                  <span className="text-muted">{t === null ? "" : formatClock(t)}</span>
                  <span className={l.severity_number >= 17 ? "text-error" : l.severity_number >= 13 ? "text-warn" : "text-muted"}>
                    {l.severity_text}
                  </span>
                  <span className="text-accent-2">{l.service_name}</span>
                  <span className="break-all">{l.body}</span>
                </li>
              );
            })}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
