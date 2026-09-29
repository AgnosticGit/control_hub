"use client";

import { useState } from "react";
import type { EngineState } from "./useWorkspaceEngine";
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
}

/** Logs, traces and events for the 5 minutes up to the selected time (side panel). */
export function SignalsPanel({ projectId, environmentId, state, selectedId, tab, onTab, traceId, onTrace }: Props) {
  const [onlySelected, setOnlySelected] = useState(true);
  // Quantize to 1 s and debounce so scrubbing/replay does not flood the API.
  const cursorSec = useDebounced(state.cursor / NS_PER_SEC, state.mode === "replay" ? 800 : 250);
  const to = (cursorSec + 1n) * NS_PER_SEC;
  const from = to - PANEL_WINDOW_NS > state.rangeFrom ? to - PANEL_WINDOW_NS : state.rangeFrom;
  const node = state.topology?.nodes.find((n) => n.id === selectedId) ?? null;
  const serviceNode = node && node.kind === "service" && onlySelected ? node : null;

  const scope: PanelScope = {
    projectId,
    environmentId,
    from: from < to ? from : to - NS_PER_SEC,
    to,
    live: state.mode === "live",
    serviceId: serviceNode?.id ?? null,
    serviceName: serviceNode?.name ?? null,
  };

  const tabs: Array<{ id: Tab; label: string }> = [
    { id: "logs", label: "Logs" },
    { id: "traces", label: "Traces" },
    { id: "events", label: "Events" },
  ];
  const showTrace = (id: string) => {
    onTrace(id);
    onTab("traces");
  };

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
      <div className="min-h-0 flex-1" role="tabpanel">
        {tab === "logs" ? <LogsPanel scope={scope} onTrace={showTrace} /> : null}
        {tab === "traces" ? <TracesPanel scope={scope} traceId={traceId} onTrace={onTrace} /> : null}
        {tab === "events" ? <EventsPanel scope={scope} onTrace={showTrace} /> : null}
      </div>
    </section>
  );
}
