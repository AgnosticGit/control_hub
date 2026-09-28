"use client";

import { useEffect, useRef, useState } from "react";
import type { Frame, TopologyView } from "@controlhub/web-engine";
import type { PixiMap } from "@/renderer/pixi-map";
import type { WorkspaceActions } from "./useWorkspaceEngine";
import { NodeTable } from "./NodeTable";

type ZoomAction = "in" | "out" | "fit";
const ZOOM_BUTTONS: Array<{ label: string; text: string; action: ZoomAction }> = [
  { label: "Zoom in", text: "+", action: "in" },
  { label: "Zoom out", text: "−", action: "out" },
  { label: "Fit to screen", text: "⤢", action: "fit" },
];

interface Props {
  actions: WorkspaceActions;
  topology: TopologyView | null;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  forceTable: boolean;
}

export function MapView({ actions, topology, selectedId, onSelect, forceTable }: Props) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<PixiMap | null>(null);
  const [webglError, setWebglError] = useState<string | null>(null);
  const [lastFrame, setLastFrame] = useState<Frame | null>(null);
  const showTable = forceTable || webglError !== null;
  const onSelectRef = useRef(onSelect);
  const topologyRef = useRef(topology);
  const selectedRef = useRef(selectedId);
  useEffect(() => {
    onSelectRef.current = onSelect;
    topologyRef.current = topology;
    selectedRef.current = selectedId;
  }, [onSelect, topology, selectedId]);

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
        const topo = topologyRef.current;
        map.setNames(topo?.nodes.map((n) => n.name) ?? []);
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
      mapRef.current?.destroy();
      mapRef.current = null;
    };
  }, [actions, showTable]);

  useEffect(() => {
    mapRef.current?.setNames(topology?.nodes.map((n) => n.name) ?? []);
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
      <div className="absolute bottom-3 right-3 flex flex-col gap-1">
        {ZOOM_BUTTONS.map((b) => (
          <button
            key={b.label}
            type="button"
            aria-label={b.label}
            title={b.label}
            onClick={() => zoomControl(b.action)}
            className="h-8 w-8 rounded-md border border-border bg-surface text-sm hover:border-accent"
          >
            {b.text}
          </button>
        ))}
      </div>
      <Legend />
    </div>
  );
}

function Legend() {
  const items = [
    { color: "bg-accent", label: "Healthy" },
    { color: "bg-warn", label: "Degraded" },
    { color: "bg-error", label: "Failing" },
    { color: "bg-slate-500", label: "No data" },
  ];
  return (
    <div className="pointer-events-none absolute left-3 top-3 rounded-md border border-border bg-surface/90 px-3 py-2 text-[11px] text-muted">
      <div className="flex gap-3">
        {items.map((i) => (
          <span key={i.label} className="flex items-center gap-1">
            <span className={`inline-block h-2 w-2 rounded-full ${i.color}`} />
            {i.label}
          </span>
        ))}
      </div>
      <div className="mt-1">Solid edge: observed · Dashed: inferred · Box: dependency</div>
    </div>
  );
}
