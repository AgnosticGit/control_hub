"use client";

import { useEffect, useState } from "react";
import type { NodeDiff, NodeMetrics, TopologyView } from "@controlhub/web-engine";
import type { EngineState, WorkspaceActions } from "./useWorkspaceEngine";
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
      <div className="p-4 text-sm text-muted">
        <p>Select a component on the map to inspect it.</p>
        <p className="mt-3 text-xs">
          Status is computed in the browser engine from traces and logs in a 30 s window ending at the selected time.
        </p>
      </div>
    );
  }

  const edges = relations(topology, node.index);

  return (
    <div className="space-y-4 p-4 text-sm">
      <div>
        <div className="flex items-start justify-between gap-2">
          <h2 className="break-all text-base font-semibold">{node.name}</h2>
          <button type="button" aria-label="Close inspector" onClick={() => onSelect(null)} className="text-muted hover:text-text">
            ✕
          </button>
        </div>
        <p className="text-xs text-muted">
          {node.kind} · {node.provenance}
        </p>
        <p className="mt-1 break-all font-mono text-[11px] text-muted">{node.id}</p>
      </div>

      {metrics ? (
        <section aria-label="Metrics at selected time">
          <p className={`font-medium ${STATUS_CLASS[metrics.status] ?? ""}`}>
            {metrics.status}
            <span className="ml-2 text-xs font-normal text-muted">{metrics.status_reason}</span>
          </p>
          <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
            <Metric label="Active now" value={metrics.active} />
            <Metric label="Requests" value={metrics.window_requests} />
            <Metric label="Completed" value={metrics.window_completed} />
            <Metric label="Errors" value={metrics.window_errors} warn={metrics.window_errors > 0} />
            <Metric label="Avg latency" value={metrics.avg_latency_ms === null ? "—" : `${metrics.avg_latency_ms.toFixed(1)} ms`} />
            <Metric label="Logs" value={metrics.window_logs} />
            <Metric label="Error logs" value={metrics.window_error_logs} warn={metrics.window_error_logs > 0} />
            <Metric label="Events" value={metrics.window_events} />
          </dl>
          <p className="mt-1 text-[11px] text-muted">
            At {formatClock(state.cursor)}; 30 s window.
          </p>
        </section>
      ) : null}

      <section aria-label="Relations">
        <h3 className="text-xs font-semibold uppercase text-muted">Relations</h3>
        {edges.length === 0 ? <p className="mt-1 text-xs text-muted">No relations observed in the loaded window.</p> : null}
        <ul className="mt-2 space-y-2">
          {edges.map((e) => (
            <li key={`${e.dir}-${e.other.id}-${e.relation}`} className="rounded border border-border p-2 text-xs">
              <div>
                {e.dir === "out" ? "→ " : "← "}
                <button type="button" className="font-medium hover:text-accent" onClick={() => onSelect(e.other.id)}>
                  {e.other.name}
                </button>
                <span className="ml-2 text-muted">
                  {e.relation} · <span className={e.provenance === "inferred" ? "text-warn" : ""}>{e.provenance}</span>
                </span>
              </div>
              {e.reasons.length > 0 ? (
                <ul className="mt-1 text-muted">
                  {e.reasons.map((r) => (
                    <li key={r} className="ml-4 list-disc">
                      {r}
                    </li>
                  ))}
                </ul>
              ) : null}
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

function Metric({ label, value, warn }: { label: string; value: number | string; warn?: boolean }) {
  return (
    <>
      <dt className="text-muted">{label}</dt>
      <dd className={`text-right tabular-nums ${warn ? "text-error" : ""}`}>{value}</dd>
    </>
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
        <button type="button" onClick={onClose} aria-label="Close comparison" className="text-muted hover:text-text">
          ✕
        </button>
      </div>
      {compare.diffs.length === 0 ? <p className="text-xs text-muted">No component changed status or traffic.</p> : null}
      <ul className="space-y-2">
        {compare.diffs.map((d) => (
          <li key={d.id} className="rounded border border-border p-2 text-xs">
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
