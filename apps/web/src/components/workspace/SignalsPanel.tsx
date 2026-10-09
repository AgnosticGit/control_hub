"use client";

import { useEffect, useState } from "react";
import type { EngineState, WorkspaceActions } from "./useWorkspaceEngine";
import { LogsPanel } from "./LogsPanel";
import { TracesPanel } from "./TracesPanel";
import { EventsPanel } from "./EventsPanel";
import { useDebounced } from "./useDebounced";
import { NS_PER_SEC, nowNs } from "@/lib/time";

export type Tab = "logs" | "traces" | "events";

export interface PanelScope {
  projectId: string;
  environmentId: string;
  /** Query window: [from, to), ending at the selected time. */
  from: bigint;
  to: bigint;
  live: boolean;
  serviceId: string | null;
  serviceName: string | null;
  displayFrom: bigint;
  displayTo: bigint;
}

const PANEL_WINDOW_NS = 5n * 60n * NS_PER_SEC;

interface Props {
  projectId: string;
  environmentId: string;
  state: EngineState;
  selectedId: string | null;
  tab: Tab;
  onTab: (t: Tab) => void;
  traceId: string | null;
  onTrace: (id: string | null) => void;
  onSelectRecord: (serviceName: string, time: bigint) => void;
  actions: WorkspaceActions;
}

/** Logs, traces and events for the 5 minutes up to the selected time (side panel). */
export function SignalsPanel({ projectId, environmentId, state, selectedId, tab, onTab, traceId, onTrace, onSelectRecord, actions }: Props) {
  const [onlySelected, setOnlySelected] = useState(true);
  const node = state.topology?.nodes.find((n) => n.id === selectedId) ?? null;
  const serviceNode = node && node.kind === "service" && onlySelected ? node : null;

  const tabs: Array<{ id: Tab; label: string }> = [
    { id: "logs", label: "Logs" },
    { id: "traces", label: "Traces" },
    { id: "events", label: "Events" },
  ];
  return (
    <section className="@container flex min-h-0 flex-1 flex-col" aria-label="Signals">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border px-3 py-2">
        <div className="seg" role="tablist">
          {tabs.map((t) => (
            <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} onClick={() => onTab(t.id)} className="seg-item">
              {t.label}
            </button>
          ))}
        </div>
        <span className="text-[11px] text-muted" title="The window follows the selected time on the timeline">
          Last 5 min to cursor
        </span>
        {node && node.kind === "service" ? (
          <label className="toggle ml-auto">
            <input type="checkbox" className="switch" checked={onlySelected} onChange={(e) => setOnlySelected(e.target.checked)} />
            Only {node.name}
          </label>
        ) : null}
      </div>
      <SignalPanelRecords
        key={state.mode === "live" ? "live" : "selected-time"}
        projectId={projectId}
        environmentId={environmentId}
        state={state}
        serviceId={serviceNode?.id ?? null}
        serviceName={serviceNode?.name ?? null}
        tab={tab}
        traceId={traceId}
        onTrace={onTrace}
        onTab={onTab}
        onSelectRecord={onSelectRecord}
        actions={actions}
      />
    </section>
  );
}

interface SignalPanelRecordsProps {
  projectId: string;
  environmentId: string;
  state: EngineState;
  serviceId: string | null;
  serviceName: string | null;
  tab: Tab;
  traceId: string | null;
  onTrace: (id: string | null) => void;
  onTab: (tab: Tab) => void;
  onSelectRecord: (serviceName: string, time: bigint) => void;
  actions: WorkspaceActions;
}

function SignalPanelRecords({
  projectId, environmentId, state, serviceId, serviceName, tab, traceId, onTrace, onTab, onSelectRecord, actions,
}: SignalPanelRecordsProps) {
  const cursorSec = useDebounced(state.cursor / NS_PER_SEC, state.mode === "replay" ? 800 : 250);
  const displayTo = (cursorSec + 1n) * NS_PER_SEC;
  const displayFrom = displayTo - PANEL_WINDOW_NS > state.rangeFrom ? displayTo - PANEL_WINDOW_NS : state.rangeFrom;
  const [liveRange, setLiveRange] = useState<{ from: bigint; to: bigint } | null>(() => {
    if (state.mode !== "live") return null;
    const to = ((cursorSec / 60n) + 1n) * 60n * NS_PER_SEC;
    return { from: to - PANEL_WINDOW_NS, to };
  });
  const queryFrom = state.mode === "live" ? (liveRange?.from ?? displayFrom) : displayFrom;
  const queryTo = state.mode === "live" ? (liveRange?.to ?? displayTo) : displayTo;
  const scope: PanelScope = {
    projectId,
    environmentId,
    from: queryFrom < queryTo ? queryFrom : queryTo - NS_PER_SEC,
    to: queryTo,
    live: state.mode === "live",
    serviceId,
    serviceName,
    displayFrom: state.mode === "live" ? displayFrom : queryFrom,
    displayTo: state.mode === "live" ? displayTo : queryTo,
  };
  const showTrace = (id: string) => { onTrace(id); onTab("traces"); };
  return (
    <div className="min-h-0 flex-1" role="tabpanel">
      {tab === "logs" ? <LogsPanel scope={scope} actions={actions} onTrace={showTrace} onSelectRecord={onSelectRecord} /> : null}
      {tab === "traces" ? <TracesPanel scope={scope} actions={actions} traceId={traceId} onTrace={onTrace} /> : null}
      {tab === "events" ? <EventsPanel scope={scope} actions={actions} onTrace={showTrace} onSelectRecord={onSelectRecord} /> : null}
    </div>
  );
}
