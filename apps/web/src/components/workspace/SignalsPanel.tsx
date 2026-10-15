"use client";

import { useEffect, useRef, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import type { EngineState, WorkspaceActions } from "./useWorkspaceEngine";
import { LogsPanel } from "./LogsPanel";
import { TracesPanel } from "./TracesPanel";
import { EventsPanel } from "./EventsPanel";
import { useDebounced } from "./useDebounced";
import { NS_PER_SEC } from "@/lib/time";

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
  const [expandedLogId, setExpandedLogId] = useState<string | null>(null);
  const [expandedEventId, setExpandedEventId] = useState<string | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const panelRef = useRef<HTMLElement | null>(null);
  useEffect(() => {
    const sync = () => setFullscreen(document.fullscreenElement === panelRef.current);
    document.addEventListener("fullscreenchange", sync);
    return () => document.removeEventListener("fullscreenchange", sync);
  }, []);
  const toggleFullscreen = () => {
    const panel = panelRef.current;
    if (!panel) return;
    if (document.fullscreenElement === panel) void document.exitFullscreen();
    else void panel.requestFullscreen().catch(() => setFullscreen(false));
  };
  const node = state.topology?.nodes.find((n) => n.id === selectedId) ?? null;
  const serviceNode = node && node.kind === "service" && onlySelected ? node : null;

  const tabs: Array<{ id: Tab; label: string }> = [
    { id: "logs", label: "Logs" },
    { id: "traces", label: "Traces" },
    { id: "events", label: "Events" },
  ];
  return (
    <section ref={panelRef} className={`signals-panel-fullscreen @container relative flex min-h-0 flex-1 flex-col ${fullscreen ? "bg-bg" : ""}`} aria-label="Signals">
      {fullscreen ? (
        <header className="shrink-0 border-b border-border bg-surface px-5 py-5 @3xl:px-8">
          <p className="text-[11px] font-semibold tracking-[0.18em] text-accent uppercase">Workspace</p>
          <h2 className="mt-1 text-2xl font-semibold text-text">Signal explorer</h2>
          <p className="mt-1 text-sm text-muted">Inspect logs, traces and events around the selected time.</p>
        </header>
      ) : null}
      <button
        type="button"
        className={`btn btn-icon absolute z-20 h-8 w-8 bg-surface/95 ${fullscreen ? "top-6 right-5 @3xl:right-8" : "top-2 right-3"}`}
        aria-label={fullscreen ? "Exit full screen signals" : "Full screen signals"}
        title={fullscreen ? "Exit full screen (Esc)" : "Full screen signals"}
        aria-pressed={fullscreen}
        onClick={toggleFullscreen}
      >
        <Icon name={fullscreen ? "minimize" : "maximize"} className="h-4 w-4" />
      </button>
      <div className={`flex min-h-0 flex-1 ${fullscreen ? "flex-col @3xl:flex-row" : "flex-col"}`}>
      <div className={fullscreen
        ? "flex shrink-0 flex-wrap items-center gap-3 border-b border-border bg-surface px-5 py-4 @3xl:w-60 @3xl:flex-col @3xl:items-stretch @3xl:justify-start @3xl:border-r @3xl:border-b-0"
        : "flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border px-3 py-2 pr-12"}>
        <div className={fullscreen ? "flex gap-1 @3xl:flex-col" : "seg"} role="tablist" aria-label="Signal type">
          {tabs.map((t) => (
            <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} onClick={() => onTab(t.id)} className={fullscreen
              ? "rounded-lg px-3 py-2 text-left text-sm font-medium text-muted hover:bg-surface-2 hover:text-text aria-selected:bg-accent/15 aria-selected:text-accent"
              : "seg-item"}>
              {t.label}
            </button>
          ))}
        </div>
        <span className="text-[11px] text-muted" title="The window follows the selected time on the timeline">
          Last 5 min to cursor
        </span>
        {node && node.kind === "service" ? (
          <label className={`toggle ${fullscreen ? "" : "ml-auto"}`}>
            <input type="checkbox" className="switch" checked={onlySelected} onChange={(e) => setOnlySelected(e.target.checked)} />
            Only {node.name}
          </label>
        ) : null}
      </div>
      <div className={fullscreen ? "min-h-0 min-w-0 flex-1 p-3 @3xl:p-6" : "min-h-0 flex-1"}>
      <div className={fullscreen ? "mx-auto h-full w-full max-w-[1500px] overflow-hidden rounded-xl border border-border bg-surface shadow-xl" : "h-full"}>
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
        expandedLogId={expandedLogId}
        onExpandedLogIdChange={setExpandedLogId}
        expandedEventId={expandedEventId}
        onExpandedEventIdChange={setExpandedEventId}
      />
      </div>
      </div>
      </div>
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
  expandedLogId: string | null;
  onExpandedLogIdChange: (id: string | null) => void;
  expandedEventId: string | null;
  onExpandedEventIdChange: (id: string | null) => void;
}

function SignalPanelRecords({
  projectId, environmentId, state, serviceId, serviceName, tab, traceId, onTrace, onTab, onSelectRecord, actions,
  expandedLogId, onExpandedLogIdChange, expandedEventId, onExpandedEventIdChange,
}: SignalPanelRecordsProps) {
  const cursorSec = useDebounced(state.cursor / NS_PER_SEC, state.mode === "replay" ? 800 : 250);
  const displayTo = (cursorSec + 1n) * NS_PER_SEC;
  const displayFrom = displayTo - PANEL_WINDOW_NS > state.rangeFrom ? displayTo - PANEL_WINDOW_NS : state.rangeFrom;
  const [liveRange] = useState<{ from: bigint; to: bigint } | null>(() => {
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
      {tab === "logs" ? <LogsPanel scope={scope} actions={actions} onTrace={showTrace} onSelectRecord={onSelectRecord} expandedId={expandedLogId} onExpandedIdChange={onExpandedLogIdChange} /> : null}
      {tab === "traces" ? <TracesPanel scope={scope} actions={actions} traceId={traceId} onTrace={onTrace} /> : null}
      {tab === "events" ? (
        <EventsPanel
          scope={scope}
          actions={actions}
          onTrace={showTrace}
          onSelectRecord={onSelectRecord}
          expandedId={expandedEventId}
          onExpandedIdChange={onExpandedEventIdChange}
        />
      ) : null}
    </div>
  );
}
