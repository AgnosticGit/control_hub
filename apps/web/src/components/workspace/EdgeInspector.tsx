"use client";

import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import type { EdgeMetrics } from "@karbird/web-engine";
import { Icon } from "@/components/ui/Icon";
import { api, scopedPath, unwrap } from "@/lib/api";
import { formatClock, nowNs } from "@/lib/time";
import type { EngineState, WorkspaceActions } from "./useWorkspaceEngine";
import { SignalLinks, type OpenSignals } from "./InfraInspector";
import {
  areaPath,
  applyLiveEdgeContribution,
  bucketRates,
  formatMs,
  formatPercent,
  formatRate,
  peakIndex,
  sizeNote,
  sizeState,
  type EdgeTraffic,
  type SelectedEdge,
} from "./edge-traffic";

const TRAFFIC_BUCKETS = 60;
const CHART_W = 320;
const CHART_H = 56;

const TONE: Record<string, string> = {
  healthy: "border-accent-2/40 bg-accent-2/10 text-accent-2",
  degraded: "border-warn/40 bg-warn/10 text-warn",
  failing: "border-error/40 bg-error/10 text-error",
  unknown: "border-border-strong bg-surface-3 text-muted",
};

interface Props {
  projectId: string;
  environmentId: string;
  state: EngineState;
  actions: WorkspaceActions;
  edge: SelectedEdge;
  onClose: () => void;
  onSelect: (id: string) => void;
  onSignals: OpenSignals;
}

export function EdgeInspector({ projectId, environmentId, state, actions, edge, onClose, onSelect, onSignals }: Props) {
  const [metrics, setMetrics] = useState<EdgeMetrics | null>(null);
  const [missing, setMissing] = useState(false);
  const queryClient = useQueryClient();
  const summaryKey = `${state.summary?.generation ?? 0}:${state.summary?.time_ns ?? ""}`;

  useEffect(() => {
    let cancelled = false;
    void actions.edgeMetrics(edge.source, edge.target).then((m) => {
      if (cancelled) return;
      setMetrics(m);
      setMissing(m === null);
    });
    return () => {
      cancelled = true;
    };
  }, [actions, edge.source, edge.target, summaryKey]);

  const relation = metrics?.relation === "depends_on" ? "depends_on" : "calls";
  // Live ranges slide every batch: refetch on a timer over "the same span up to now" instead.
  const live = state.mode === "live";
  const spanNs = state.rangeTo - state.rangeFrom;
  const rangeKey = live ? `live:${spanNs}` : `${state.rangeFrom}-${state.rangeTo}`;
  const trafficKey = useMemo(() => ["edge-traffic", projectId, environmentId, edge.source, edge.target, relation, rangeKey] as const,
    [projectId, environmentId, edge.source, edge.target, relation, rangeKey]);
  const traffic = useQuery({
    queryKey: trafficKey,
    enabled: metrics !== null,
    queryFn: async () => {
      const to = live ? nowNs() : state.rangeTo;
      const from = live ? to - spanNs : state.rangeFrom;
      return unwrap(
        await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/edges/traffic", {
          params: {
            path: scopedPath(projectId, environmentId),
            query: {
              source: edge.source,
              target: edge.target,
              relation,
              from: from.toString(),
              to: to.toString(),
              buckets: TRAFFIC_BUCKETS,
            },
          },
        }),
      );
    },
    placeholderData: keepPreviousData,
    refetchInterval: false,
  });

  const accumulatorKey = `${edge.source}:${edge.target}:${relation}`;
  const accumulatorIdentity = `${accumulatorKey}:${rangeKey}`;
  const baselineCursor = traffic.data?.resume_cursor_ns;
  const hasTraffic = traffic.data !== undefined;
  const hasMetrics = metrics !== null;
  const baseline = useRef<{ key: string; cursor: string } | null>(null);
  const seenChanges = useRef(new Set<string>());
  useEffect(() => {
    if (baselineCursor !== undefined && baseline.current?.key !== accumulatorIdentity) {
      baseline.current = { key: accumulatorIdentity, cursor: baselineCursor };
      seenChanges.current.clear();
    }
  }, [accumulatorIdentity, baselineCursor]);

  useEffect(() => {
    const initial = baseline.current;
    if (!live || !hasMetrics || !hasTraffic || initial?.key !== accumulatorIdentity) return;
    const unsubscribe = actions.onEdgeChange((change) => {
      if (change.source !== edge.source || change.target !== edge.target || change.relation !== relation) return;
      if (seenChanges.current.has(change.id)) return;
      seenChanges.current.add(change.id);
      if (seenChanges.current.size > 20_000) {
        const oldest = seenChanges.current.values().next().value;
        if (oldest !== undefined) seenChanges.current.delete(oldest);
      }
      const contribution = {
        id: change.id,
        timeNs: BigInt(change.time_ns),
        requestBytes: change.request_bytes,
        responseBytes: change.response_bytes,
        storageCursor: BigInt(change.storage_cursor_ns),
      };
      queryClient.setQueryData<EdgeTraffic>(trafficKey, (current) =>
        current ? applyLiveEdgeContribution(current, contribution, BigInt(current.to_ns)) : current,
      );
    });
    void actions.subscribeEdge(edge.source, edge.target, relation, initial.cursor);
    return () => {
      unsubscribe();
      void actions.unsubscribeEdge();
    };
  }, [actions, accumulatorIdentity, edge.source, edge.target, hasMetrics, hasTraffic, live, queryClient, relation, trafficKey]);

  const sourceName = metrics?.source_name ?? state.topology?.nodes.find((n) => n.id === edge.source)?.name ?? edge.source;
  const targetName = metrics?.target_name ?? state.topology?.nodes.find((n) => n.id === edge.target)?.name ?? edge.target;
  const kindOf = (id: string) => state.topology?.nodes.find((n) => n.id === id)?.kind;
  const signalService = kindOf(edge.target) === "service" ? edge.target : kindOf(edge.source) === "service" ? edge.source : null;

  return (
    <article className="text-sm" aria-label="Link details">
      <div className="flex items-start gap-3 border-b border-border p-4">
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-accent/10 text-accent">
          <Icon name="arrowRight" className="h-5 w-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="text-base font-semibold break-all">
            {sourceName} → {targetName}
          </h2>
          <p className="text-xs text-muted">
            link{metrics ? ` · ${metrics.relation} · ${metrics.provenance}` : ""}
          </p>
        </div>
        <button type="button" aria-label="Close link" onClick={onClose} className="btn btn-icon btn-sm">
          <Icon name="x" className="h-3.5 w-3.5" />
        </button>
      </div>

      <div className="space-y-4 p-4">
        {missing ? <p className="text-xs text-muted">This link is not on the map in the loaded range.</p> : null}
        {metrics ? <EngineStats metrics={metrics} cursor={state.cursor} /> : null}
        {metrics ? <TrafficSection traffic={traffic.data ?? null} loading={traffic.isPending} failed={traffic.isError} /> : null}

        <div className="flex flex-wrap gap-2" role="group" aria-label="Link ends">
          <button type="button" className="btn btn-sm" onClick={() => onSelect(edge.source)}>
            <Icon name="arrowLeft" className="h-3.5 w-3.5" />
            {sourceName}
          </button>
          <button type="button" className="btn btn-sm" onClick={() => onSelect(edge.target)}>
            {targetName}
            <Icon name="arrowRight" className="h-3.5 w-3.5" />
          </button>
        </div>
        <SignalLinks serviceId={signalService} onSignals={onSignals} label={`${sourceName} → ${targetName}`} />

        {metrics && metrics.reasons.length > 0 ? (
          <section aria-label="Why this link exists">
            <h3 className="label">Why this link exists</h3>
            <ul className="mt-1 text-xs text-muted">
              {metrics.reasons.map((r) => (
                <li key={r} className="ml-4 list-disc">
                  {r}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
      </div>
    </article>
  );
}

function EngineStats({ metrics: m, cursor }: { metrics: EdgeMetrics; cursor: bigint }) {
  const windowS = Number(BigInt(m.window_ns) / 1_000_000_000n);
  const spark = m.calls_series;
  return (
    <section aria-label="Calls at selected time" className="space-y-3">
      <div className="flex items-center gap-2">
        <span className={`badge ${TONE[m.status] ?? TONE.unknown}`}>
          <span className="dot bg-current" />
          {m.status}
        </span>
        <span className="text-xs text-muted">{m.status_reason}</span>
      </div>
      <dl className="grid grid-cols-3 gap-2">
        <Stat label="Calls" value={m.window_calls} />
        <Stat label="Calls/min" value={m.calls_per_min.toFixed(m.calls_per_min >= 100 ? 0 : 1)} />
        <Stat label="In flight" value={m.active} />
        <Stat label="Errors" value={m.window_errors} warn={m.window_errors > 0} />
        <Stat label="Error rate" value={m.error_ratio === null ? "—" : formatPercent(m.error_ratio)} warn={(m.error_ratio ?? 0) > 0} />
        <Stat label="Latency avg" value={m.latency ? formatMs(m.latency.avg_ms) : "—"} />
        <Stat label="p50" value={m.latency ? formatMs(m.latency.p50_ms) : "—"} />
        <Stat label="p95" value={m.latency ? formatMs(m.latency.p95_ms) : "—"} />
        <Stat label="Max" value={m.latency ? formatMs(m.latency.max_ms) : "—"} />
      </dl>
      <figure aria-label="Calls in the window" className="rounded-lg border border-border bg-bg/50 px-3 py-2">
        <svg viewBox={`0 0 ${CHART_W} 28`} className="h-7 w-full" preserveAspectRatio="none" aria-hidden>
          <path d={areaPath(spark, CHART_W, 28)} className="fill-accent/30 stroke-accent" strokeWidth="1" />
          <path d={areaPath(m.errors_series, CHART_W, 28)} className="fill-error/40" />
        </svg>
        <figcaption className="mt-1 text-[11px] text-muted">
          Calls (and errors) over the {windowS} s window ending {formatClock(cursor)}.
        </figcaption>
      </figure>
    </section>
  );
}

function TrafficSection({ traffic: t, loading, failed }: { traffic: EdgeTraffic | null; loading: boolean; failed: boolean }) {
  if (failed && !t) {
    return (
      <p role="alert" className="text-xs text-error">
        Traffic could not be loaded.
      </p>
    );
  }
  if (!t) return loading ? <p className="text-xs text-muted">Loading traffic…</p> : null;
  const s = sizeState(t);
  const known = s.kind === "partial" || s.kind === "full";
  const rates = bucketRates(t);
  const peak = peakIndex(rates);
  const peakX = peak === null || rates.length === 0 ? null : ((peak + 0.5) / rates.length) * CHART_W;
  const peakAt = t.peak_response_at_ns ?? t.peak_request_at_ns;
  return (
    <section aria-label="Data flow" className="space-y-2">
      <h3 className="label">Data flow</h3>
      {known ? (
        <>
          <dl className="grid grid-cols-2 gap-2">
            <Stat label="↑ Requests avg" value={formatRate(t.avg_request_bps)} />
            <Stat label="↓ Responses avg" value={formatRate(t.avg_response_bps)} />
            <Stat label="↑ Requests peak" value={formatRate(t.peak_request_bps)} />
            <Stat label="↓ Responses peak" value={formatRate(t.peak_response_bps)} />
          </dl>
          <figure aria-label="Data flow over the loaded range" className="rounded-lg border border-border bg-bg/50 px-3 py-2">
            <svg viewBox={`0 0 ${CHART_W} ${CHART_H}`} className="h-14 w-full" preserveAspectRatio="none" aria-hidden>
              <path d={areaPath(rates, CHART_W, CHART_H)} className="fill-accent-2/25 stroke-accent-2" strokeWidth="1" />
              {peakX !== null ? <line x1={peakX} x2={peakX} y1={0} y2={CHART_H} className="stroke-warn" strokeDasharray="3 2" /> : null}
            </svg>
            <figcaption className="mt-1 text-[11px] text-muted">
              Bytes per second, requests and responses together{peakAt ? `; peak at ${formatClock(BigInt(peakAt))}` : ""}.
            </figcaption>
          </figure>
        </>
      ) : null}
      <p className="text-xs text-muted">
        {sizeNote(s, {
          calls: t.total_calls,
          requestSizedCalls: t.total_request_sized_calls,
          responseSizedCalls: t.total_response_sized_calls,
        })}
      </p>
    </section>
  );
}

function Stat({ label, value, warn }: { label: string; value: number | string; warn?: boolean }) {
  return (
    <div className="rounded-lg border border-border bg-bg/50 px-2.5 py-1.5">
      <dt className="text-[11px] text-muted">{label}</dt>
      <dd className={`text-base leading-tight font-semibold tabular-nums ${warn ? "text-error" : ""}`}>{value}</dd>
    </div>
  );
}
