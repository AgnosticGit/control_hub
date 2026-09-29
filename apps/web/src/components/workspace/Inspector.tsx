"use client";

import { useEffect, useState } from "react";
import type { NodeDiff, NodeMetrics, TopologyView } from "@controlhub/web-engine";
import type { EngineState, WorkspaceActions } from "./useWorkspaceEngine";
import { Icon } from "@/components/ui/Icon";
import { formatClock } from "@/lib/time";

const STATUS_CLASS: Record<string, string> = {
  healthy: "text-accent",
  degraded: "text-warn",
  failing: "text-error",
  unknown: "text-muted",
};

interface Props {
  state: EngineState;
  actions: WorkspaceActions;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  compare: { a: bigint; b: bigint; diffs: NodeDiff[] } | null;
  onCloseCompare: () => void;
}

export function Inspector({ state, actions, selectedId, onSelect, compare, onCloseCompare }: Props) {
  const [metrics, setMetrics] = useState<NodeMetrics | null>(null);
  const topology = state.topology;
  const node = topology?.nodes.find((n) => n.id === selectedId) ?? null;
  const summaryKey = `${state.summary?.generation ?? 0}:${state.summary?.time_ns ?? ""}`;

  useEffect(() => {
    if (!selectedId) return;
    let cancelled = false;
    void actions.nodeMetrics(selectedId).then((m) => {
      if (!cancelled) setMetrics(m);
    });
    return () => {
      cancelled = true;
    };
  }, [actions, selectedId, summaryKey]);

  if (compare) return <CompareView compare={compare} onClose={onCloseCompare} onSelect={onSelect} />;

  if (!node) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center text-sm text-muted">
        <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-accent/10 text-accent">
          <Icon name="layers" className="h-6 w-6" />
        </span>
        <p className="text-text">Select a component on the map to inspect it.</p>
        <p className="text-xs">
          Status is computed in the browser engine from traces and logs in a 30 s window ending at the selected time.
        </p>
      </div>
    );
  }

  const edges = relations(topology, node.index);
  const tone = metrics ? (STATUS_TONE[metrics.status] ?? STATUS_TONE.unknown) : STATUS_TONE.unknown;

  return (
    <div className="text-sm">
      <div className="flex items-start gap-3 border-b border-border p-4">
        <span className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-xl ${tone?.box ?? ""}`}>
          <Icon name={node.kind === "service" ? "server" : "database"} className="h-5 w-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="text-base font-semibold break-all">{node.name}</h2>
          <p className="text-xs text-muted">
            {node.kind} · {node.provenance}
          </p>
        </div>
        <button type="button" aria-label="Close inspector" onClick={() => onSelect(null)} className="btn btn-icon btn-sm">
          <Icon name="x" className="h-3.5 w-3.5" />
        </button>
      </div>

      <div className="space-y-4 p-4">
        {metrics ? (
          <section aria-label="Metrics at selected time" className="space-y-3">
            <div className="flex items-center gap-2">
              <span className={`badge ${tone?.badge ?? ""}`}>
                <span className="dot bg-current" />
                {metrics.status}
              </span>
              <span className="text-xs text-muted">{metrics.status_reason}</span>
            </div>
            <dl className="grid grid-cols-2 gap-2">
              <Metric label="Active now" value={metrics.active} />
              <Metric label="Requests" value={metrics.window_requests} />
              <Metric label="Completed" value={metrics.window_completed} />
              <Metric label="Errors" value={metrics.window_errors} warn={metrics.window_errors > 0} />
              <Metric label="Avg latency" value={metrics.avg_latency_ms === null ? "—" : `${metrics.avg_latency_ms.toFixed(1)} ms`} />
              <Metric label="Logs" value={metrics.window_logs} />
              <Metric label="Error logs" value={metrics.window_error_logs} warn={metrics.window_error_logs > 0} />
              <Metric label="Events" value={metrics.window_events} />
            </dl>
            <p className="text-[11px] text-muted">At {formatClock(state.cursor)}; 30 s window.</p>
          </section>
        ) : null}

        <section aria-label="Relations">
          <h3 className="label">Relations</h3>
          {edges.length === 0 ? <p className="mt-1 text-xs text-muted">No relations observed in the loaded window.</p> : null}
          <ul className="mt-2 space-y-2">
            {edges.map((e) => (
              <li key={`${e.dir}-${e.other.id}-${e.relation}`} className="rounded-lg border border-border bg-bg/50 p-2.5 text-xs">
                <div className="flex items-center gap-1.5">
                  <Icon name={e.dir === "out" ? "arrowRight" : "arrowLeft"} className="h-3.5 w-3.5 text-muted" />
                  <button type="button" className="font-medium hover:text-accent" onClick={() => onSelect(e.other.id)}>
                    {e.other.name}
                  </button>
                  <span className="ml-auto text-muted">
                    {e.relation} · <span className={e.provenance === "inferred" ? "text-warn" : ""}>{e.provenance}</span>
                  </span>
                </div>
                {e.reasons.length > 0 ? (
                  <ul className="mt-1 text-muted">
                    {e.reasons.map((r) => (
                      <li key={r} className="ml-5 list-disc">
                        {r}
                      </li>
                    ))}
                  </ul>
                ) : null}
              </li>
            ))}
          </ul>
        </section>

        <p className="font-mono text-[10px] break-all text-muted">{node.id}</p>
      </div>
    </div>
  );
}

const STATUS_TONE: Record<string, { box: string; badge: string }> = {
  healthy: { box: "bg-accent-2/15 text-accent-2", badge: "border-accent-2/40 bg-accent-2/10 text-accent-2" },
  degraded: { box: "bg-warn/15 text-warn", badge: "border-warn/40 bg-warn/10 text-warn" },
  failing: { box: "bg-error/15 text-error", badge: "border-error/40 bg-error/10 text-error" },
  unknown: { box: "bg-surface-3 text-muted", badge: "border-border-strong bg-surface-3 text-muted" },
};

function Metric({ label, value, warn }: { label: string; value: number | string; warn?: boolean }) {
  return (
    <div className="rounded-lg border border-border bg-bg/50 px-3 py-2">
      <dt className="text-[11px] text-muted">{label}</dt>
      <dd className={`text-lg leading-tight font-semibold tabular-nums ${warn ? "text-error" : ""}`}>{value}</dd>
    </div>
  );
}

function relations(topology: TopologyView | null, index: number) {
  if (!topology) return [];
  const out: Array<{
    dir: "in" | "out";
    other: { id: string; name: string };
    relation: string;
    provenance: string;
    reasons: string[];
  }> = [];
  for (const e of topology.edges) {
    if (e.source !== index && e.target !== index) continue;
    const dir = e.source === index ? "out" : "in";
    const other = topology.nodes[dir === "out" ? e.target : e.source];
    if (!other) continue;
    out.push({ dir, other, relation: e.relation, provenance: e.provenance, reasons: e.reasons });
  }
  return out;
}

function CompareView({
  compare,
  onClose,
  onSelect,
}: {
  compare: { a: bigint; b: bigint; diffs: NodeDiff[] };
  onClose: () => void;
  onSelect: (id: string) => void;
}) {
  return (
    <div className="space-y-3 p-4 text-sm">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">
          Compare {formatClock(compare.a)} ↔ {formatClock(compare.b)}
        </h2>
        <button type="button" onClick={onClose} aria-label="Close comparison" className="btn btn-icon btn-sm">
          <Icon name="x" className="h-3.5 w-3.5" />
        </button>
      </div>
      {compare.diffs.length === 0 ? <p className="text-xs text-muted">No component changed status or traffic.</p> : null}
      <ul className="space-y-2">
        {compare.diffs.map((d) => (
          <li key={d.id} className="rounded-lg border border-border bg-bg/50 p-2.5 text-xs">
            <button type="button" className="font-medium hover:text-accent" onClick={() => onSelect(d.id)}>
              {d.name}
            </button>
            <div className="mt-1 grid grid-cols-3 gap-1 text-muted">
              <span />
              <span>A</span>
              <span>B</span>
              <span>Status</span>
              <span className={STATUS_CLASS[d.before.status]}>{d.before.status}</span>
              <span className={STATUS_CLASS[d.after.status]}>{d.after.status}</span>
              <span>Requests</span>
              <span>{d.before.window_requests}</span>
              <span>{d.after.window_requests}</span>
              <span>Errors</span>
              <span>{d.before.window_errors}</span>
              <span>{d.after.window_errors}</span>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
