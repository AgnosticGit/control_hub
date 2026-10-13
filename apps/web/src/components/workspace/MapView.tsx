"use client";

import { useEffect, useRef, useState } from "react";
import type { Frame, FrameSummary, InfrastructureSnapshot, TopologyView } from "@karbird/web-engine";
import { Icon } from "@/components/ui/Icon";
import type { EdgeStyle } from "@/renderer/edge-path";
import type { PixiMap } from "@/renderer/pixi-map";
import { edgeEnds, edgeIndex, type SelectedEdge } from "./edge-traffic";
import { setEdgeStyle, useEdgeStyle } from "./edgeStyleStore";
import { LIVE_ANIM_DELAY_MS, animClockFor, type Mode, type Speed, type WorkspaceActions } from "./useWorkspaceEngine";
import { MapKey } from "./MapKey";
import { NodeTable } from "./NodeTable";
import type { WorkspaceMemory } from "./workspace-memory";
import { toggleSelection } from "@/renderer/selection-style";
import { mapLayerAvailability, type MapLayerAvailability } from "@/renderer/map-layer-availability";

/** Icon paths in a 20 × 12 box: the same two cards joined in each style. */
const LINE_STYLES: Array<{ style: EdgeStyle; label: string; path: string }> = [
  { style: "curved", label: "Curved lines", path: "M2 10 Q10 -2 18 2" },
  { style: "orthogonal", label: "Square lines", path: "M2 10 H10 V2 H18" },
  { style: "metro", label: "Metro lines", path: "M2 10 H6 L14 2 H18" },
];

interface Props {
  actions: WorkspaceActions;
  topology: TopologyView | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  forceTable: boolean;
  mode: Mode;
  speed: Speed;
  summary: FrameSummary | null;
  memory: WorkspaceMemory;
  /** A click on a machine frame of the infrastructure layer. */
  onSelectHost: (hostId: string | null) => void;
  /** Machine open in the inspector; its frame is highlighted. */
  selectedHost?: string | null;
  /** A click on a service instance chip of the infrastructure layer. */
  onSelectInstance?: (instanceId: string, hostId: string, serviceId: string) => void;
  /** A click on a line between two cards. */
  onSelectEdge?: (edge: SelectedEdge | null) => void;
  /** Line open in the inspector; it is highlighted. */
  selectedEdge?: SelectedEdge | null;
  /** Screen-space height reserved below the Machines overlay when fitting the map. */
  topInset?: number;
  infrastructure: boolean;
  onInfrastructureChange: (enabled: boolean) => void;
  serviceMetrics: boolean;
  serviceMetricSnapshot: InfrastructureSnapshot | null;
  serviceMetricsLoading: boolean;
  onServiceMetricsChange: (enabled: boolean) => void;
  onlyProblems: boolean;
  onOnlyProblemsChange: (enabled: boolean) => void;
  hideInferred: boolean;
  onHideInferredChange: (enabled: boolean) => void;
  tableView: boolean;
  onTableViewChange: (enabled: boolean) => void;
  fullscreen: boolean;
  onToggleFullscreen: () => void;
}

export function MapView({
  actions,
  topology,
  selectedId,
  onSelect,
  forceTable,
  mode,
  speed,
  summary,
  memory,
  onSelectHost,
  selectedHost = null,
  onSelectInstance,
  onSelectEdge,
  selectedEdge = null,
  topInset = 0,
  infrastructure,
  onInfrastructureChange,
  serviceMetrics,
  serviceMetricSnapshot,
  serviceMetricsLoading,
  onServiceMetricsChange,
  onlyProblems,
  onOnlyProblemsChange,
  hideInferred,
  onHideInferredChange,
  tableView,
  onTableViewChange,
  fullscreen,
  onToggleFullscreen,
}: Props) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<PixiMap | null>(null);
  const [webglError, setWebglError] = useState<string | null>(null);
  const [lastFrame, setLastFrame] = useState<Frame | null>(null);
  const [availability, setAvailability] = useState<MapLayerAvailability>({ ready: false, problems: false, inferred: false });
  const showTable = forceTable || webglError !== null;
  const onSelectRef = useRef(onSelect);
  const onSelectHostRef = useRef(onSelectHost);
  const topologyRef = useRef(topology);
  const selectedRef = useRef(selectedId);
  const clockRef = useRef(animClockFor(mode, speed));
  const edgeStyle = useEdgeStyle();
  const edgeStyleRef = useRef(edgeStyle);
  const selectedHostRef = useRef(selectedHost);
  const serviceMetricsRef = useRef(serviceMetrics);
  const serviceMetricSnapshotRef = useRef(serviceMetricSnapshot);
  const serviceMetricsLoadingRef = useRef(serviceMetricsLoading);
  const topInsetRef = useRef(topInset);
  useEffect(() => {
    topInsetRef.current = topInset;
    mapRef.current?.setTopInset(topInset);
  }, [topInset]);
  useEffect(() => {
    selectedHostRef.current = selectedHost;
    mapRef.current?.setSelectedHost(selectedHost);
  }, [selectedHost]);
  useEffect(() => {
    serviceMetricsRef.current = serviceMetrics;
    mapRef.current?.setServiceMetricsEnabled(serviceMetrics);
  }, [serviceMetrics]);
  useEffect(() => {
    serviceMetricSnapshotRef.current = serviceMetricSnapshot;
    mapRef.current?.setServiceMetricSnapshot(serviceMetricSnapshot);
  }, [serviceMetricSnapshot]);
  useEffect(() => {
    serviceMetricsLoadingRef.current = serviceMetricsLoading;
    mapRef.current?.setServiceMetricsLoading(serviceMetricsLoading);
  }, [serviceMetricsLoading]);
  const onSelectEdgeRef = useRef(onSelectEdge);
  const onSelectInstanceRef = useRef(onSelectInstance);
  useEffect(() => {
    onSelectInstanceRef.current = onSelectInstance;
  }, [onSelectInstance]);
  const selectedEdgeIndex = edgeIndex(topology, selectedEdge);
  const selectedEdgeRef = useRef(selectedEdgeIndex);
  useEffect(() => {
    onSelectEdgeRef.current = onSelectEdge;
  }, [onSelectEdge]);
  useEffect(() => {
    selectedEdgeRef.current = selectedEdgeIndex;
    mapRef.current?.setSelectedEdge(selectedEdgeIndex);
  }, [selectedEdgeIndex]);
  useEffect(() => {
    if (!availability.ready) return;
    if (!availability.problems && onlyProblems) onOnlyProblemsChange(false);
    if (!availability.inferred && hideInferred) onHideInferredChange(false);
  }, [availability, hideInferred, onHideInferredChange, onOnlyProblemsChange, onlyProblems]);
  useEffect(() => {
    edgeStyleRef.current = edgeStyle;
    mapRef.current?.setEdgeStyle(edgeStyle);
  }, [edgeStyle]);
  useEffect(() => {
    onSelectRef.current = onSelect;
    onSelectHostRef.current = onSelectHost;
    topologyRef.current = topology;
    selectedRef.current = selectedId;
  }, [onSelect, onSelectHost, topology, selectedId]);

  useEffect(() => {
    clockRef.current = animClockFor(mode, speed);
    mapRef.current?.setClock(clockRef.current);
  }, [mode, speed]);

  // Renderer lifecycle: pixi.js is imported only in the browser, on demand.
  useEffect(() => {
    if (showTable) return;
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;
    let off: (() => void) | null = null;
    (async () => {
      try {
        const { PixiMap } = await import("@/renderer/pixi-map");
        if (disposed) return;
        const map = new PixiMap({
          onViewport: (x, y, w, h, s) => actions.setViewport(x, y, w, h, s),
          onPick: (x, y) => {
            void actions.pick(x, y).then((id) => onSelectRef.current(toggleSelection(selectedRef.current, id)));
          },
          onPickHost: (hostId) => onSelectHostRef.current(toggleSelection(selectedHostRef.current, hostId)),
          onPickInstance: (instanceId, hostId, serviceId) => {
            if (toggleSelection(selectedRef.current, serviceId) === null) {
              onSelectRef.current(null);
              return;
            }
            const pickInstance = onSelectInstanceRef.current;
            if (pickInstance) pickInstance(instanceId, hostId, serviceId);
            else onSelectRef.current(serviceId);
          },
          onPickEdge: (index) => {
            const edge = edgeEnds(topologyRef.current, index);
            if (edge) onSelectEdgeRef.current?.(toggleSelection(selectedEdgeRef.current, index) === null ? null : edge);
          },
        });
        await map.mount(host);
        if (disposed) {
          map.destroy();
          return;
        }
        mapRef.current = map;
        map.setTopInset(topInsetRef.current);
        const camera = memory.camera();
        if (camera) map.setCamera(camera);
        const topo = topologyRef.current;
        map.setClock(clockRef.current);
        map.setEdgeStyle(edgeStyleRef.current);
        map.setSelectedHost(selectedHostRef.current);
        map.setServiceMetricsEnabled(serviceMetricsRef.current);
        map.setServiceMetricSnapshot(serviceMetricSnapshotRef.current);
        map.setServiceMetricsLoading(serviceMetricsLoadingRef.current);
        map.setSelectedEdge(selectedEdgeRef.current);
        map.setTopology(topo);
        const sel = selectedRef.current;
        map.setSelected(sel ? (topo?.nodes.find((n) => n.id === sel)?.index ?? null) : null);
        off = actions.onFrame((f) => {
          if (f.topology) map.setTopology(f.topology);
          const nextAvailability = mapLayerAvailability(f, (f.topology ?? topologyRef.current)?.edges ?? null);
          setAvailability((current) =>
            current.ready === nextAvailability.ready && current.problems === nextAvailability.problems && current.inferred === nextAvailability.inferred
              ? current
              : nextAvailability,
          );
          map.render(f);
        });
      } catch (e) {
        if (!disposed) setWebglError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      disposed = true;
      off?.();
      if (mapRef.current) memory.saveCamera(mapRef.current.getCamera());
      mapRef.current?.destroy();
      mapRef.current = null;
    };
  }, [actions, showTable, memory]);

  useEffect(() => {
    mapRef.current?.setTopology(topology);
  }, [topology]);

  useEffect(() => {
    const idx = selectedId ? (topology?.nodes.find((n) => n.id === selectedId)?.index ?? null) : null;
    mapRef.current?.setSelected(idx);
  }, [selectedId, topology]);

  // Table view needs frames as React state (throttled).
  useEffect(() => {
    if (!showTable) return;
    // A table has no camera. Non-positive dimensions explicitly disable engine culling;
    // otherwise saved cards outside the previous map viewport disappear from its rows.
    actions.setViewport(0, 0, 0, 0, 1);
    let last = 0;
    return actions.onFrame((f) => {
      const nextAvailability = mapLayerAvailability(f, (f.topology ?? topologyRef.current)?.edges ?? null);
      setAvailability((current) =>
        current.ready === nextAvailability.ready && current.problems === nextAvailability.problems && current.inferred === nextAvailability.inferred
          ? current
          : nextAvailability,
      );
      const now = performance.now();
      if (now - last > 250) {
        last = now;
        setLastFrame(f);
      }
    });
  }, [actions, showTable]);

  function onKeyDown(e: React.KeyboardEvent) {
    const m = mapRef.current;
    if (!m) return;
    const step = 60;
    const handled: Record<string, () => void> = {
      "+": () => m.zoomBy(1.2),
      "=": () => m.zoomBy(1.2),
      "-": () => m.zoomBy(1 / 1.2),
      "0": () => m.fit(),
      ArrowLeft: () => m.panBy(step, 0),
      ArrowRight: () => m.panBy(-step, 0),
      ArrowUp: () => m.panBy(0, step),
      ArrowDown: () => m.panBy(0, -step),
      Escape: () => {
        if (!fullscreen) onSelect(null);
      },
    };
    const fn = handled[e.key];
    if (fn) {
      e.preventDefault();
      fn();
    }
  }

  if (showTable) {
    return (
      <div className="relative h-full w-full">
        <h1 className="absolute top-3 left-3 z-10 rounded-lg bg-surface/90 px-2.5 py-1.5 text-sm font-semibold text-text shadow-lg backdrop-blur">System Map</h1>
        <div className="absolute inset-x-0 bottom-0 top-10 overflow-auto">
          {webglError ? (
            <p role="status" className="border-b border-border bg-surface px-4 py-2 text-xs text-warn">
              WebGL is unavailable ({webglError}). Showing the table view.
            </p>
          ) : null}
          <NodeTable frame={lastFrame} topology={topology} selectedId={selectedId} onSelect={onSelect} />
        </div>
        <AnimationNote
          mode={mode}
          summary={summary}
          infrastructure={infrastructure}
          onInfrastructureChange={onInfrastructureChange}
          serviceMetrics={serviceMetrics}
          onServiceMetricsChange={onServiceMetricsChange}
          onlyProblems={onlyProblems}
          onOnlyProblemsChange={onOnlyProblemsChange}
          hideInferred={hideInferred}
          onHideInferredChange={onHideInferredChange}
          tableView={tableView}
          onTableViewChange={onTableViewChange}
          availability={availability}
        />
      </div>
    );
  }

  return (
    <div className="relative h-full w-full">
      <h1 className="pointer-events-none absolute top-3 left-3 z-10 rounded-lg border border-border bg-surface/90 px-2.5 py-1.5 text-sm font-semibold text-text shadow-lg backdrop-blur">
        System Map
      </h1>
      <div className="absolute inset-0">
      <div
        ref={hostRef}
        tabIndex={0}
        role="application"
        aria-label="Service map. Use arrow keys to pan, plus and minus keys or the mouse wheel to zoom, 0 to fit. Switch to table view for a list."
        onKeyDown={onKeyDown}
        className="absolute inset-0 overflow-hidden outline-none"
      />
      <div
        className="pointer-events-none absolute inset-x-3 flex items-start justify-between gap-2 transition-[top] duration-200 ease-out"
        style={{ top: Math.max(42, topInset + 16) }}
      >
        <div className="pointer-events-auto flex flex-col gap-1.5">
          <button
            type="button"
            aria-label={fullscreen ? "Exit full screen" : "Full screen"}
            title={fullscreen ? "Exit full screen (Esc)" : "Full screen"}
            aria-pressed={fullscreen}
            onClick={onToggleFullscreen}
            className="btn btn-icon h-8 w-8 bg-surface/95 shadow-lg backdrop-blur"
          >
            <Icon name={fullscreen ? "minimize" : "maximize"} className="h-4 w-4" />
          </button>
          <div role="group" aria-label="Line style" className="flex w-8 flex-col gap-0.5 rounded-lg border border-border bg-surface/90 p-0.5 backdrop-blur">
            {LINE_STYLES.map((s) => {
              const active = s.style === edgeStyle;
              return (
                <button
                  key={s.style}
                  type="button"
                  aria-label={s.label}
                  aria-pressed={active}
                  title={s.label}
                  onClick={() => setEdgeStyle(s.style)}
                  className={`flex h-7 w-full items-center justify-center rounded-md transition-colors ${
                    active ? "bg-accent/15 text-accent" : "text-muted hover:bg-surface-2 hover:text-text"
                  }`}
                >
                  <svg viewBox="0 0 20 12" className="h-3 w-5" fill="none" aria-hidden>
                    <path d={s.path} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
                  </svg>
                </button>
              );
            })}
          </div>
        </div>
      </div>
      </div>
      <div className="pointer-events-none absolute inset-x-3 bottom-3 flex justify-end">
        <Legend />
      </div>
      <AnimationNote
        mode={mode}
        summary={summary}
        infrastructure={infrastructure}
        onInfrastructureChange={onInfrastructureChange}
        serviceMetrics={serviceMetrics}
        onServiceMetricsChange={onServiceMetricsChange}
        onlyProblems={onlyProblems}
        onOnlyProblemsChange={onOnlyProblemsChange}
        hideInferred={hideInferred}
        onHideInferredChange={onHideInferredChange}
        tableView={tableView}
        onTableViewChange={onTableViewChange}
        availability={availability}
      />
    </div>
  );
}

function Legend() {
  const status = [
    { color: "bg-accent-2", label: "Healthy" },
    { color: "bg-warn", label: "Degraded" },
    { color: "bg-error", label: "Failing" },
    { color: "bg-muted", label: "No data" },
  ];
  return (
    <div className="ml-auto flex flex-wrap items-center justify-end gap-x-3 gap-y-1 rounded-xl border border-border bg-surface/85 px-3 py-2 text-[11px] text-muted backdrop-blur">
      {status.map((i) => (
        <span key={i.label} className="flex items-center gap-1.5">
          <span className={`dot ${i.color}`} />
          {i.label}
        </span>
      ))}
      <span className="h-3 w-px bg-border-strong" />
      <span className="flex items-center gap-1.5">
        <span className="inline-block h-0.5 w-4 bg-accent" /> Observed
      </span>
      <span className="flex items-center gap-1.5">
        <span className="inline-block h-0.5 w-4 border-t-2 border-dashed border-muted" /> Inferred
      </span>
      <span className="flex items-center gap-1.5">
        <span className="inline-block h-2 w-2 rounded-full bg-accent-2 shadow-[0_0_6px_var(--color-accent-2)]" /> Request
      </span>
      <span className="flex items-center gap-1.5">
        <span className="inline-block h-2 w-2 rounded-full border-2 border-accent-2 bg-surface" /> Response
      </span>
    </div>
  );
}

/** States exactly what the moving dots show, so nobody mistakes them for decoration. */
function AnimationNote({
  mode,
  summary,
  infrastructure,
  onInfrastructureChange,
  serviceMetrics,
  onServiceMetricsChange,
  onlyProblems,
  onOnlyProblemsChange,
  hideInferred,
  onHideInferredChange,
  tableView,
  onTableViewChange,
  availability,
}: {
  mode: Mode;
  summary: FrameSummary | null;
  infrastructure: boolean;
  onInfrastructureChange: (enabled: boolean) => void;
  serviceMetrics: boolean;
  onServiceMetricsChange: (enabled: boolean) => void;
  onlyProblems: boolean;
  onOnlyProblemsChange: (enabled: boolean) => void;
  hideInferred: boolean;
  onHideInferredChange: (enabled: boolean) => void;
  tableView: boolean;
  onTableViewChange: (enabled: boolean) => void;
  availability: MapLayerAvailability;
}) {
  const clock =
    mode === "live"
      ? `${LIVE_ANIM_DELAY_MS / 1000} s behind live (telemetry is delivered in batches)`
      : mode === "replay"
        ? "replay at a readable pace"
        : "frozen at the selected time";
  const capped = summary && summary.hops_shown < summary.hops_in_window ? `Showing ${summary.hops_shown} of ${summary.hops_in_window} requests` : null;
  return (
    <div className="pointer-events-none absolute top-3 right-3 z-20 flex items-center gap-2">
      {capped ? (
        <span className="rounded-lg border border-warn/40 bg-surface/90 px-2 py-1 text-[11px] text-warn backdrop-blur">{capped}</span>
      ) : null}
      <MapLayers
        infrastructure={infrastructure}
        onInfrastructureChange={onInfrastructureChange}
        serviceMetrics={serviceMetrics}
        onServiceMetricsChange={onServiceMetricsChange}
        onlyProblems={onlyProblems}
        onOnlyProblemsChange={onOnlyProblemsChange}
        hideInferred={hideInferred}
        onHideInferredChange={onHideInferredChange}
        tableView={tableView}
        onTableViewChange={onTableViewChange}
        availability={availability}
      />
      <MapKey animation={`Requests from recorded spans, ${clock}.`} capped={capped} />
    </div>
  );
}

function MapLayers({
  infrastructure,
  onInfrastructureChange,
  serviceMetrics,
  onServiceMetricsChange,
  onlyProblems,
  onOnlyProblemsChange,
  hideInferred,
  onHideInferredChange,
  tableView,
  onTableViewChange,
  availability,
}: {
  infrastructure: boolean;
  onInfrastructureChange: (enabled: boolean) => void;
  serviceMetrics: boolean;
  onServiceMetricsChange: (enabled: boolean) => void;
  onlyProblems: boolean;
  onOnlyProblemsChange: (enabled: boolean) => void;
  hideInferred: boolean;
  onHideInferredChange: (enabled: boolean) => void;
  tableView: boolean;
  onTableViewChange: (enabled: boolean) => void;
  availability: MapLayerAvailability;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && !rootRef.current?.contains(event.target)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div className="pointer-events-auto relative" ref={rootRef}>
      <button
        type="button"
        className={`flex h-8 w-8 items-center justify-center rounded-lg border bg-surface/90 backdrop-blur transition-colors hover:text-text ${
          open ? "border-accent/60 text-accent" : "border-border text-muted"
        }`}
        aria-label="Map layers"
        aria-expanded={open}
        aria-controls="map-layers-panel"
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="layers" className="h-4 w-4" />
      </button>
      {open ? (
        <div
          id="map-layers-panel"
          role="group"
          aria-label="Map layers"
          className="absolute top-10 right-0 z-30 w-64 rounded-xl border border-border bg-surface-2 p-3 shadow-xl"
        >
          <p className="mb-2 px-2 text-xs font-semibold tracking-wide text-muted uppercase">Map layers</p>
          <div className="space-y-1">
            <LayerToggle label="Infrastructure" checked={infrastructure} onChange={onInfrastructureChange} />
            <LayerToggle label="Service metrics" checked={serviceMetrics} onChange={onServiceMetricsChange} />
            <LayerToggle label="Only problems" checked={onlyProblems} onChange={onOnlyProblemsChange} disabled={!availability.ready || !availability.problems} />
            <LayerToggle label="Hide inferred" checked={hideInferred} onChange={onHideInferredChange} disabled={!availability.ready || !availability.inferred} />
            <LayerToggle label="Table view" checked={tableView} onChange={onTableViewChange} />
          </div>
        </div>
      ) : null}
    </div>
  );
}

function LayerToggle({ label, checked, onChange, disabled = false }: { label: string; checked: boolean; onChange: (enabled: boolean) => void; disabled?: boolean }) {
  return (
    <label className={`flex items-center justify-between gap-4 rounded-lg px-2 py-2 text-sm text-text ${disabled ? "cursor-not-allowed opacity-45" : "cursor-pointer hover:bg-bg/70"}`}>
      {label}
      <input type="checkbox" className="switch" checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
    </label>
  );
}
