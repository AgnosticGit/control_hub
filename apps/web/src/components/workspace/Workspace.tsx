"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { NODE_STRIDE, type Diagnostics, type NodeDiff } from "@karbird/web-engine";
import { AppShell, StatusRow } from "@/components/shell/AppShell";
import { Icon, type IconName } from "@/components/ui/Icon";
import { useSession } from "@/lib/session";
import { formatClock, parseNs } from "@/lib/time";
import { Splitter } from "@/components/ui/Splitter";
import { usePersisted, writePersisted } from "@/lib/persisted";
import type { SelectedEdge } from "./edge-traffic";
import { Inspector } from "./Inspector";
import type { OpenSignals } from "./InfraInspector";
import { InfraOverlay } from "./InfraOverlay";
import type { ContainerRef } from "./infrastructure";
import { MapView } from "./MapView";
import { useInfrastructure } from "./useInfrastructure";
import { SignalsPanel, type Tab } from "./SignalsPanel";
import { TimelineBar } from "./TimelineBar";
import { TraceFocusBar } from "./TraceFocusBar";
import type { View } from "./timeline/scale";
import { useWorkspaceEngine, type EngineState, type WorkspaceActions } from "./useWorkspaceEngine";
import { WorkspaceMemory } from "./workspace-memory";

type SideTab = "details" | "signals";
const SIDE_TABS: Array<{ id: SideTab; label: string; icon: IconName }> = [
  { id: "details", label: "Details", icon: "info" },
  { id: "signals", label: "Signals", icon: "list" },
];
const SIDE_MIN = 320;
const SIDE_MAX = 760;
const DOCK_MIN = 150;
const DOCK_MAX = 640;
const DOCK_MAP_RESERVE = 360;

interface MapLayers {
  infrastructure: boolean;
  onlyProblems: boolean;
  hideInferred: boolean;
  tableView: boolean;
}

const DEFAULT_MAP_LAYERS: MapLayers = { infrastructure: false, onlyProblems: false, hideInferred: false, tableView: false };

function parseMapLayers(raw: string): MapLayers | null {
  try {
    const value: unknown = JSON.parse(raw);
    if (typeof value !== "object" || value === null) return null;
    const record = value as Record<string, unknown>;
    if (
      typeof record.infrastructure !== "boolean" ||
      typeof record.onlyProblems !== "boolean" ||
      typeof record.hideInferred !== "boolean" ||
      typeof record.tableView !== "boolean"
    ) {
      return null;
    }
    return {
      infrastructure: record.infrastructure,
      onlyProblems: record.onlyProblems,
      hideInferred: record.hideInferred,
      tableView: record.tableView,
    };
  } catch {
    return null;
  }
}

/** A remembered panel size in px, clamped to [min, max]. */
function usePersistedSize(key: string, fallback: number, min: number, max: number): [number, (v: number) => void] {
  const value = usePersisted(key, fallback, (raw) => {
    const n = Number(raw);
    return Number.isFinite(n) ? Math.max(min, Math.min(max, n)) : null;
  });
  return [value, (v: number) => writePersisted(key, String(Math.round(v)))];
}

export function Workspace({ projectId, environmentId, visible = true }: { projectId: string; environmentId: string; visible?: boolean }) {
  const session = useSession();
  const { state, actions } = useWorkspaceEngine(projectId, environmentId);
  const layerPrefsKey = `karbird.workspace.mapLayers.${projectId}.${environmentId}`;
  const layerPrefs = usePersisted(layerPrefsKey, DEFAULT_MAP_LAYERS, parseMapLayers);
  // Survives the map and timeline unmounting while another page is shown.
  const [memory] = useState(() => new WorkspaceMemory());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("logs");
  const [traceId, setTraceId] = useState<string | null>(null);
  // With a selected request: show only it on the map and the timeline.
  const [traceOnly, setTraceOnly] = useState(true);
  const [traceFocus, setTraceFocus] = useState<View | null>(null);
  // Service node IDs switched off on the timeline: dimmed on the map and left out of totals.
  const [muted, setMuted] = useState<string[]>([]);
  const [selectedHost, setSelectedHost] = useState<string | null>(null);
  const [selectedEdge, setSelectedEdge] = useState<SelectedEdge | null>(null);
  const [selectedContainer, setSelectedContainer] = useState<ContainerRef | null>(null);
  const [overlayBottom, setOverlayBottom] = useState(0);
  const mapPanelRef = useRef<HTMLElement | null>(null);
  const [mapFullscreen, setMapFullscreen] = useState(false);
  const [markA, setMarkA] = useState<bigint | null>(null);
  const [compare, setCompare] = useState<{ a: bigint; b: bigint; diffs: NodeDiff[] } | null>(null);
  const [side, setSide] = useState<SideTab>("signals");
  const [sideWidth, setSideWidth] = usePersistedSize("karbird.workspace.sideWidth", 400, SIDE_MIN, SIDE_MAX);
  const [dockHeight, setDockHeight] = usePersistedSize("karbird.workspace.timelineHeight", 220, DOCK_MIN, DOCK_MAX);
  const [dockContentHeight, setDockContentHeight] = useState(DOCK_MAX);
  const [dockAvailableHeight, setDockAvailableHeight] = useState(DOCK_MAX);
  const workspaceRef = useRef<HTMLDivElement | null>(null);
  const dockMax = Math.max(DOCK_MIN, Math.min(DOCK_MAX, dockContentHeight, dockAvailableHeight));
  const selectedNode = state.topology?.nodes.find((n) => n.id === selectedId) ?? null;
  const infra = useInfrastructure(actions, state.phase === "ready", layerPrefs.infrastructure, state.cursor);
  const infraSnapshot = layerPrefs.infrastructure ? infra.snapshot : null;
  const changeLayer = (layer: keyof MapLayers, enabled: boolean) =>
    writePersisted(layerPrefsKey, JSON.stringify({ ...layerPrefs, [layer]: enabled }));

  useEffect(() => {
    const element = workspaceRef.current;
    if (!element) return;
    const resize = () => setDockAvailableHeight(Math.max(DOCK_MIN, element.clientHeight - DOCK_MAP_RESERVE));
    const observer = new ResizeObserver(resize);
    observer.observe(element);
    resize();
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (dockHeight > dockMax) setDockHeight(dockMax);
  }, [dockHeight, dockMax, setDockHeight]);

  useEffect(() => {
    const syncFullscreen = () => setMapFullscreen(document.fullscreenElement === mapPanelRef.current);
    const exitOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && document.fullscreenElement === mapPanelRef.current) {
        void document.exitFullscreen();
      }
    };
    document.addEventListener("fullscreenchange", syncFullscreen);
    document.addEventListener("keydown", exitOnEscape, true);
    return () => {
      document.removeEventListener("fullscreenchange", syncFullscreen);
      document.removeEventListener("keydown", exitOnEscape, true);
    };
  }, []);

  const toggleMapFullscreen = () => {
    const panel = mapPanelRef.current;
    if (!panel) return;
    if (document.fullscreenElement === panel) {
      void document.exitFullscreen();
    } else {
      void panel.requestFullscreen().catch(() => setMapFullscreen(false));
    }
  };

  // Selecting something opens its details; the signals tab stays one click away.
  const select = (id: string | null) => {
    setSelectedId(id);
    setSelectedHost(null);
    setSelectedEdge(null);
    setSelectedContainer(null);
    if (id !== null) setSide("details");
  };
  const selectHost = (id: string | null) => {
    setSelectedHost(id);
    setSelectedContainer(null);
    if (id !== null) {
      setSelectedId(null);
      setSelectedEdge(null);
      setSide("details");
    }
  };
  const selectEdge = (edge: SelectedEdge | null) => {
    setSelectedEdge(edge);
    if (edge !== null) {
      setSelectedId(null);
      setSelectedHost(null);
      setSelectedContainer(null);
      setSide("details");
    }
  };
  const selectContainer = (ref: ContainerRef | null) => {
    setSelectedContainer(ref);
    if (ref !== null) {
      setSelectedId(null);
      setSelectedHost(null);
      setSelectedEdge(null);
      setSide("details");
    }
  };
  // A replica chip opens the service inspector, which lists every replica and its own metrics.
  const selectInstance = (_instanceId: string, _hostId: string, serviceId: string) => select(serviceId);
  // Traces or logs of the moment on screen: time stops there so the lists match the map.
  const openSignals: OpenSignals = (signal, serviceId) => {
    if (state.mode === "live" || state.mode === "replay") actions.pause();
    if (serviceId !== null) setSelectedId(serviceId);
    setTab(signal);
    setSide("signals");
  };

  const project = session.data?.projects.find((p) => p.id === projectId);
  const env = project?.environments.find((e) => e.id === environmentId);

  useEffect(() => {
    if (state.phase !== "ready") return;
    actions.setFilters({
      search: "",
      hide_inferred: layerPrefs.hideInferred,
      only_problems: layerPrefs.onlyProblems,
      selected_trace_id: traceId,
      trace_only: traceOnly,
      muted_services: muted,
      infrastructure: layerPrefs.infrastructure,
    });
  }, [actions, state.phase, layerPrefs, traceId, traceOnly, muted]);

  const selectTrace = (id: string | null) => {
    setTraceId(id);
    if (id !== null) setTraceOnly(true);
  };
  const closeTrace = () => {
    setTraceId(null);
    setTraceFocus(null);
    actions.setLoop(null);
    actions.pause();
  };

  if (!visible) return null;
  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  if (session.data && !env) {
    return (
      <main className="flex min-h-screen items-center justify-center p-8">
        <div className="card max-w-md p-6 text-sm">
          <p>This environment does not exist or you do not have access to it.</p>
          <Link href="/app" className="btn mt-4">
            <Icon name="arrowLeft" className="h-3.5 w-3.5" />
            Back to environments
          </Link>
        </div>
      </main>
    );
  }

  const status = (
    <>
      <LiveTransportRow transport={state.liveTransport} />
      <CompletenessRow state={state} />
      <EngineInfo state={state} diagnostics={actions.diagnostics} />
    </>
  );

  return (
    <AppShell projectId={projectId} environmentId={environmentId} active="overview" status={status}>
      <div ref={workspaceRef} className="flex h-full min-h-0 flex-col p-3">
        <KpiRow state={state} actions={actions} />

        <div className="flex min-h-0 flex-1">
          <main ref={mapPanelRef} className="system-map-fullscreen card relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden" aria-label="System map">
            <div className="sr-only" role="region" aria-label="System Map text">
              {state.topology?.nodes.map((node) => <span key={node.id}>{node.name}</span>)}
            </div>
            <div className="card-header">
              <span className="card-title-icon">
                <Icon name="layers" className="h-3.5 w-3.5" />
              </span>
              <h1>System Map</h1>
            </div>
            <div className="relative min-h-0 flex-1">
              {state.phase === "error" ? (
                <div role="alert" className="p-6 text-sm text-error">
                  Engine failed: {state.error}
                </div>
              ) : (
                <MapView
                  actions={actions}
                  topology={state.topology}
                  selectedId={selectedId}
                  onSelect={select}
                  forceTable={layerPrefs.tableView}
                  mode={state.mode}
                  speed={state.speed}
                  summary={state.summary}
                  memory={memory}
                  onSelectHost={selectHost}
                  selectedHost={layerPrefs.infrastructure ? selectedHost : null}
                  onSelectInstance={selectInstance}
                  onSelectEdge={selectEdge}
                  selectedEdge={selectedEdge}
                  topInset={layerPrefs.infrastructure && !layerPrefs.tableView ? overlayBottom : 0}
                  infrastructure={layerPrefs.infrastructure}
                  onInfrastructureChange={(enabled) => {
                    changeLayer("infrastructure", enabled);
                    if (!enabled) {
                      setSelectedHost(null);
                      setSelectedContainer(null);
                    }
                  }}
                  onlyProblems={layerPrefs.onlyProblems}
                  onOnlyProblemsChange={(enabled) => changeLayer("onlyProblems", enabled)}
                  hideInferred={layerPrefs.hideInferred}
                  onHideInferredChange={(enabled) => changeLayer("hideInferred", enabled)}
                  tableView={layerPrefs.tableView}
                  onTableViewChange={(enabled) => changeLayer("tableView", enabled)}
                  fullscreen={mapFullscreen}
                  onToggleFullscreen={toggleMapFullscreen}
                />
              )}
              {layerPrefs.infrastructure && !layerPrefs.tableView && state.phase === "ready" ? (
                <InfraOverlay
                  snapshot={infraSnapshot}
                  error={infra.error}
                  selectedHost={selectedHost}
                  visibleHostIds={state.topology?.infra?.frames.map((frame) => frame.host_id) ?? []}
                  onlyProblems={layerPrefs.onlyProblems}
                  onSelectHost={selectHost}
                  onSelect={select}
                  onBottom={setOverlayBottom}
                />
              ) : null}
              {traceId && state.phase === "ready" ? (
                <TraceFocusBar
                  key={traceId}
                  projectId={projectId}
                  environmentId={environmentId}
                  traceId={traceId}
                  traceOnly={traceOnly}
                  onTraceOnly={setTraceOnly}
                  onClose={closeTrace}
                  onFocus={setTraceFocus}
                  state={state}
                  actions={actions}
                />
              ) : null}
              {state.phase === "starting" ? (
                <div className="absolute inset-0 flex items-center justify-center text-sm text-muted">Starting engine…</div>
              ) : null}
              {state.phase === "ready" && state.topology && state.topology.nodes.length === 0 && !state.loading ? (
                <div className="absolute inset-0 flex items-center justify-center p-6 text-center text-sm text-muted">
                  No telemetry in this time range yet. Start the demo with <code className="mx-1 font-mono">just demo</code> or
                  connect a service.
                </div>
              ) : null}
              {state.error && state.phase === "ready" ? (
                <div role="alert" className="absolute top-14 left-3 max-w-md rounded-lg border border-error/50 bg-surface px-3 py-2 text-xs text-error">
                  {state.error}
                </div>
              ) : null}
            </div>
          </main>

          <Splitter axis="column" label="Resize side panel" value={sideWidth} min={SIDE_MIN} max={SIDE_MAX} onChange={setSideWidth} />

          <div className="card flex min-h-0 shrink-0 flex-col overflow-hidden" style={{ width: sideWidth }}>
            <div className="flex items-center gap-2 border-b border-border px-3 py-2">
              <div className="seg" role="tablist" aria-label="Side panel">
                {SIDE_TABS.map((t) => (
                  <button
                    key={t.id}
                    type="button"
                    role="tab"
                    aria-selected={side === t.id}
                    onClick={() => setSide(t.id)}
                    className="seg-item flex items-center gap-1.5"
                  >
                    <Icon name={t.icon} className="h-3.5 w-3.5" />
                    {t.label}
                  </button>
                ))}
              </div>
              {side === "signals" && selectedNode ? (
                <button type="button" className="ml-auto truncate text-[11px] text-muted hover:text-accent" onClick={() => setSide("details")}>
                  {selectedNode.name} →
                </button>
              ) : null}
            </div>
            {side === "details" ? (
              <aside className="min-h-0 flex-1 overflow-auto" aria-label="Inspector">
                <Inspector
                  projectId={projectId}
                  environmentId={environmentId}
                  state={state}
                  actions={actions}
                  selectedId={selectedId}
                  selectedEdge={selectedEdge}
                  onSelectEdge={selectEdge}
                  onSelect={select}
                  compare={compare}
                  onCloseCompare={() => setCompare(null)}
                  infra={infraSnapshot}
                  selectedHost={selectedHost}
                  onSelectHost={selectHost}
                  selectedContainer={selectedContainer}
                  onSelectContainer={selectContainer}
                  onSignals={openSignals}
                />
              </aside>
            ) : (
              <SignalsPanel
                projectId={projectId}
                environmentId={environmentId}
                state={state}
                selectedId={selectedId}
                tab={tab}
                onTab={setTab}
                traceId={traceId}
                onTrace={selectTrace}
                onSelectRecord={(serviceName, time) => {
                  if (state.mode === "live" || state.mode === "replay") actions.pause();
                  const service = state.topology?.nodes.find((node) => node.kind === "service" && node.name === serviceName);
                  if (service) setSelectedId(service.id);
                  actions.seek(time);
                }}
              />
            )}
          </div>
        </div>

        <Splitter axis="row" label="Resize bottom dock" value={dockHeight} min={DOCK_MIN} max={dockMax} onChange={setDockHeight} />

        <TimelineBar
          projectId={projectId}
          environmentId={environmentId}
          filterKey={`${layerPrefs.hideInferred}|${layerPrefs.onlyProblems}|${muted.join(",")}|${traceId ?? ""}|${traceOnly}`}
          muted={muted}
          onMuted={setMuted}
          focus={traceFocus}
          height={dockHeight}
          memory={memory}
          onContentHeight={setDockContentHeight}
          state={state}
          actions={actions}
          markA={markA}
          onMarkA={setMarkA}
          onCompare={(a, b) => {
            void actions.compare(a, b).then((diffs) => {
              setCompare({ a, b, diffs });
              setSide("details");
            });
          }}
        />
      </div>
    </AppShell>
  );
}

/** Service counts by health from the latest frame (throttled). */
function useHealthCounts(actions: WorkspaceActions) {
  const [counts, setCounts] = useState({ degraded: 0, failing: 0 });
  useEffect(() => {
    let last = 0;
    return actions.onFrame((f) => {
      const now = performance.now();
      if (now - last < 500) return;
      last = now;
      let degraded = 0;
      let failing = 0;
      for (let i = 0; i < f.nodeIndices.length; i++) {
        const s = f.nodeData[i * NODE_STRIDE + 3] ?? 0;
        if (s === 2) degraded++;
        else if (s === 3) failing++;
      }
      setCounts((c) => (c.degraded === degraded && c.failing === failing ? c : { degraded, failing }));
    });
  }, [actions]);
  return counts;
}

function KpiRow({ state, actions }: { state: EngineState; actions: WorkspaceActions }) {
  const health = useHealthCounts(actions);
  const s = state.summary;
  const services = state.topology?.nodes.filter((n) => n.kind === "service").length ?? 0;
  const req = s?.window_requests ?? 0;
  const active = s?.total_active ?? 0;
  const err = s?.window_errors ?? 0;
  const unhealthy = health.degraded + health.failing;
  const rate = req > 0 ? (err / req) * 100 : 0;
  return (
    <div className="mb-3 grid shrink-0 grid-cols-2 gap-3 xl:grid-cols-4">
      <Kpi icon="server" tone="ok" label="Services" value={String(services)} sub={unhealthy > 0 ? `${unhealthy} unhealthy` : "all healthy or idle"} subTone={unhealthy > 0 ? "error" : "muted"} progress={services ? ((services - unhealthy) / services) * 100 : 0} />
      <Kpi icon="activity" tone="accent" label="Requests / min" value={String(req * 2)} sub={`${req} in the last 30 s`} progress={Math.min(100, (req * 2 / 2_000) * 100)} />
      <Kpi icon="alert" tone="error" label="Error rate" value={`${rate.toFixed(1)}%`} sub={`${err} failed in the last 30 s`} subTone={err > 0 ? "error" : "muted"} progress={Math.min(100, rate * 10)} />
      <Kpi icon="radio" tone="violet" label="Active requests" value={String(active)} sub={state.mode === "live" ? "completed spans available at cursor" : "in flight at the selected time"} progress={Math.min(100, active * 12.5)} title="Calculated from spans that cross the selected timestamp. SDKs export spans after completion, so short requests usually arrive after they have ended; this does not measure requests currently executing on a live service." />
    </div>
  );
}

const TONE: Record<string, string> = {
  ok: "bg-ok/15 text-ok",
  accent: "bg-accent/15 text-accent",
  error: "bg-error/15 text-error",
  violet: "bg-violet/15 text-violet",
};

function Kpi({
  icon,
  tone,
  label,
  value,
  sub,
  subTone = "muted",
  progress,
  title,
}: {
  icon: IconName;
  tone: keyof typeof TONE;
  label: string;
  value: string;
  sub: string;
  subTone?: "muted" | "error";
  progress: number;
  title?: string;
}) {
  const ringColor = { ok: "var(--color-ok)", accent: "var(--color-accent)", error: "var(--color-error)", violet: "var(--color-violet)" }[tone];
  return (
    <div className="card flex items-center gap-3 px-3 py-2" title={title}>
      <span className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ${TONE[tone]}`}>
        <Icon name={icon} className="h-4 w-4" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-[11px] text-muted">{label}</div>
        <div className="flex items-baseline gap-2">
          <span className="text-lg leading-tight font-semibold tabular-nums">{value}</span>
          <span className={`truncate text-[11px] ${subTone === "error" ? "text-error" : "text-muted"}`}>{sub}</span>
        </div>
      </div>
      <span
        className="relative h-9 w-9 shrink-0 rounded-full"
        style={{ background: `conic-gradient(${ringColor} ${Math.max(0, Math.min(100, progress))}%, var(--color-surface-3) 0)` }}
        aria-hidden
      >
        <span className="absolute inset-[5px] rounded-full bg-surface" />
      </span>
    </div>
  );
}

const TRANSPORT: Record<Exclude<EngineState["liveTransport"], "off">, { text: string; title: string; tone: "ok" | "warn" | "muted" }> = {
  connecting: { text: "Live: connecting", title: "Opening the live WebSocket", tone: "muted" },
  websocket: { text: "Live: WebSocket", title: "New telemetry is pushed over a WebSocket", tone: "ok" },
  polling: { text: "Live: polling", title: "WebSocket unavailable; polling the API every 2 s until it reconnects", tone: "warn" },
};

function LiveTransportRow({ transport }: { transport: EngineState["liveTransport"] }) {
  if (transport === "off") {
    return (
      <StatusRow tone="muted">
        <span className="text-muted">Live channel off</span>
      </StatusRow>
    );
  }
  const t = TRANSPORT[transport];
  return (
    <StatusRow tone={t.tone}>
      <span title={t.title} aria-label="Live transport">
        {t.text}
      </span>
    </StatusRow>
  );
}

function CompletenessRow({ state }: { state: EngineState }) {
  const c = state.load?.completeness;
  if (!state.load) return null;
  const problems: string[] = [];
  if (state.load.truncated) problems.push("Timeline truncated: more data exists than loaded.");
  if (c && !c.processing_caught_up) problems.push("Processing is behind ingestion.");
  if (c && c.unresolved_processing_errors > 0) problems.push(`${c.unresolved_processing_errors} record(s) failed processing.`);
  const watermark = parseNs(c?.watermark_ns);
  const title = [
    ...problems,
    ...(c?.notes ?? []),
    watermark ? `Processed up to ${formatClock(watermark)}` : "No processing watermark yet",
    `Sampling: ${c?.sampling ?? "unknown"}`,
  ].join("\n");
  return (
    <StatusRow tone={problems.length ? "warn" : "ok"}>
      <span title={title} className={problems.length ? "text-warn" : ""}>
        {problems.length ? "Data may be incomplete" : "Data complete for range"}
      </span>
    </StatusRow>
  );
}

function EngineInfo({ state, diagnostics }: { state: EngineState; diagnostics: () => Promise<Diagnostics | null> }) {
  const [open, setOpen] = useState(false);
  const [diag, setDiag] = useState<Diagnostics | null>(null);
  return (
    <div className="relative">
      <StatusRow tone={state.phase === "error" ? "error" : state.phase === "ready" ? "ok" : "muted"}>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => {
            setOpen((v) => !v);
            void diagnostics().then(setDiag);
          }}
          className="flex w-full items-center justify-between hover:text-accent"
        >
          Engine
          <span className="font-mono text-[10px] text-muted">{state.frameMs.toFixed(1)} ms</span>
        </button>
      </StatusRow>
      {open ? (
        <div className="absolute bottom-full left-0 z-20 mb-2 w-72 rounded-xl border border-border-strong bg-surface-2 p-3 font-mono text-[11px] shadow-2xl">
          <Row k="WASM init" v={state.init ? `${state.init.initMs.toFixed(0)} ms` : "—"} />
          <Row k="WASM size" v={state.init ? `${(state.init.wasmBytes / 1024).toFixed(0)} KiB` : "—"} />
          <Row k="Last frame" v={`${state.frameMs.toFixed(2)} ms`} />
          <Row k="Last fetch / apply" v={state.load ? `${state.load.fetchMs.toFixed(0)} / ${state.load.applyMs.toFixed(1)} ms` : "—"} />
          <Row k="Last batch items" v={String(state.load?.items ?? 0)} />
          <Row k="Hops in frame" v={state.summary ? `${state.summary.hops_shown} / ${state.summary.hops_in_window}` : "—"} />
          {diag ? (
            <>
              <Row k="Spans / logs / events" v={`${diag.spans} / ${diag.logs} / ${diag.events}`} />
              <Row k="Nodes / edges" v={`${diag.nodes} / ${diag.edges}`} />
              <Row k="Checkpoints" v={`${diag.checkpoints} (every ${diag.checkpoint_interval})`} />
              <Row k="Timeline rebuilds / merges" v={`${diag.timeline_rebuilds} / ${diag.timeline_merges}`} />
              <Row k="Approx memory" v={`${(diag.approx_memory_bytes / 1048576).toFixed(1)} MiB`} />
              <Row k="Duplicates ignored" v={String(diag.duplicates_ignored)} />
              <Row k="Evicted" v={String(diag.evicted_items)} />
              <Row k="Unresolved parents" v={String(diag.unresolved_parents)} />
              <Row k="Stale seeks dropped" v={String(diag.stale_seeks_ignored)} />
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between gap-2 py-0.5">
      <span className="text-muted">{k}</span>
      <span>{v}</span>
    </div>
  );
}
