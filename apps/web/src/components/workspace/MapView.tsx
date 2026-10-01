"use client";

import { useEffect, useRef, useState } from "react";
import type { Frame, FrameSummary, TopologyView } from "@controlhub/web-engine";
import { Icon, type IconName } from "@/components/ui/Icon";
import type { EdgeStyle } from "@/renderer/edge-path";
import type { PixiMap } from "@/renderer/pixi-map";
import { setEdgeStyle, useEdgeStyle } from "./edgeStyleStore";
import { LIVE_ANIM_DELAY_MS, animClockFor, type Mode, type Speed, type WorkspaceActions } from "./useWorkspaceEngine";
import { MapKey } from "./MapKey";
import { NodeTable } from "./NodeTable";
import type { WorkspaceMemory } from "./workspace-memory";

type ZoomAction = "in" | "out" | "fit";
const ZOOM_BUTTONS: Array<{ label: string; icon: IconName; action: ZoomAction }> = [
  { label: "Zoom in", icon: "plus", action: "in" },
  { label: "Zoom out", icon: "minus", action: "out" },
  { label: "Fit to screen", icon: "maximize", action: "fit" },
];

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
}

export function MapView({ actions, topology, selectedId, onSelect, forceTable, mode, speed, summary, memory }: Props) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<PixiMap | null>(null);
  const [webglError, setWebglError] = useState<string | null>(null);
  const [lastFrame, setLastFrame] = useState<Frame | null>(null);
  const showTable = forceTable || webglError !== null;
  const onSelectRef = useRef(onSelect);
  const topologyRef = useRef(topology);
  const selectedRef = useRef(selectedId);
  const clockRef = useRef(animClockFor(mode, speed));
  const edgeStyle = useEdgeStyle();
  const edgeStyleRef = useRef(edgeStyle);
  useEffect(() => {
    edgeStyleRef.current = edgeStyle;
    mapRef.current?.setEdgeStyle(edgeStyle);
  }, [edgeStyle]);
  useEffect(() => {
    onSelectRef.current = onSelect;
    topologyRef.current = topology;
    selectedRef.current = selectedId;
  }, [onSelect, topology, selectedId]);

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
            void actions.pick(x, y).then((id) => onSelectRef.current(id));
          },
        });
        await map.mount(host);
        if (disposed) {
          map.destroy();
          return;
        }
        mapRef.current = map;
        const camera = memory.camera();
        if (camera) map.setCamera(camera);
        const topo = topologyRef.current;
        map.setClock(clockRef.current);
        map.setEdgeStyle(edgeStyleRef.current);
        map.setTopology(topo);
        const sel = selectedRef.current;
        map.setSelected(sel ? (topo?.nodes.find((n) => n.id === sel)?.index ?? null) : null);
        off = actions.onFrame((f) => map.render(f));
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
    let last = 0;
    return actions.onFrame((f) => {
      const now = performance.now();
      if (now - last > 250) {
        last = now;
        setLastFrame(f);
      }
    });
  }, [actions, showTable]);

  function zoomControl(action: ZoomAction) {
    const m = mapRef.current;
    if (!m) return;
    if (action === "in") m.zoomBy(1.2);
    else if (action === "out") m.zoomBy(1 / 1.2);
    else m.fit();
  }

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
      Escape: () => onSelect(null),
    };
    const fn = handled[e.key];
    if (fn) {
      e.preventDefault();
      fn();
    }
  }

  if (showTable) {
    return (
      <div className="h-full overflow-auto">
        {webglError ? (
          <p role="status" className="border-b border-border bg-surface px-4 py-2 text-xs text-warn">
            WebGL is unavailable ({webglError}). Showing the table view.
          </p>
        ) : null}
        <NodeTable frame={lastFrame} topology={topology} selectedId={selectedId} onSelect={onSelect} />
      </div>
    );
  }

  return (
    <div className="relative h-full w-full">
      <div
        ref={hostRef}
        tabIndex={0}
        role="application"
        aria-label="Service map. Use arrow keys to pan, plus and minus to zoom, 0 to fit. Switch to table view for a list."
        onKeyDown={onKeyDown}
        className="absolute inset-0 overflow-hidden outline-none"
      />
      <div className="pointer-events-none absolute inset-x-3 bottom-3 flex flex-wrap-reverse items-end justify-between gap-2">
        <div className="pointer-events-auto flex gap-1.5">
          {ZOOM_BUTTONS.map((b) => (
            <button
              key={b.label}
              type="button"
              aria-label={b.label}
              title={b.label}
              onClick={() => zoomControl(b.action)}
              className="btn btn-icon bg-surface/90 backdrop-blur"
            >
              <Icon name={b.icon} className="h-4 w-4" />
            </button>
          ))}
          <div role="group" aria-label="Line style" className="ml-1.5 flex gap-0.5 rounded-lg border border-border bg-surface/90 p-0.5 backdrop-blur">
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
                  className={`flex h-7 w-8 items-center justify-center rounded-md transition-colors ${
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
        <Legend />
      </div>
      <AnimationNote mode={mode} speed={speed} summary={summary} />
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
function AnimationNote({ mode, speed, summary }: { mode: Mode; speed: Speed; summary: FrameSummary | null }) {
  const clock =
    mode === "live"
      ? `${LIVE_ANIM_DELAY_MS / 1000} s behind live (telemetry is delivered in batches)`
      : mode === "replay"
        ? `replay at ${speed}×${speed <= 0.001 ? " — 1 ms of recorded time per second" : ""}`
        : "frozen at the selected time";
  const capped = summary && summary.hops_shown < summary.hops_in_window ? `Showing ${summary.hops_shown} of ${summary.hops_in_window} requests` : null;
  return (
    <div className="pointer-events-none absolute top-3 right-3 flex items-center gap-2">
      {capped ? (
        <span className="rounded-lg border border-warn/40 bg-surface/90 px-2 py-1 text-[11px] text-warn backdrop-blur">{capped}</span>
      ) : null}
      <MapKey animation={`Requests from recorded spans, ${clock}.`} capped={capped} />
    </div>
  );
}
