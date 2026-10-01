"use client";

import { useQuery } from "@tanstack/react-query";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { TimelineTracks } from "@controlhub/web-engine";
import { Icon } from "@/components/ui/Icon";
import { api, scopedPath, unwrap } from "@/lib/api";
import { RESOURCE_COLOR, cssColor, serviceColors } from "@/renderer/service-colors";
import { NS_PER_MS, formatClock, formatClockMicros, formatDateTime } from "@/lib/time";
import type { EngineState, WorkspaceActions } from "../useWorkspaceEngine";
import type { WorkspaceMemory } from "../workspace-memory";
import { RULER_H, bucketGeometry, draw, drawRuler, rowTop, totalHeight, type ActionBar, type Row } from "./draw";
import { placeAbove, rangeBetween, viewAround } from "./range";
import {
  clampView,
  follow,
  followLive,
  formatSpan,
  futureShare,
  hasFuture,
  panBy,
  timeAt,
  withFuture,
  xOf,
  zoomAt,
  zoomCentered,
  type View,
} from "./scale";

const BUCKET_PX = 3;
const CLIP_CAP = 400;
const REFETCH_MS = 100;
export const LABEL_W = 168;
/** Pointer travel (px) that turns a click on the tracks into a range drag. */
const DRAG_PX = 4;
/** Distance (px) from a range edge at which dragging moves that edge. */
const EDGE_PX = 6;

interface Props {
  projectId: string;
  environmentId: string;
  state: EngineState;
  actions: WorkspaceActions;
  markA: bigint | null;
  /** Changes whenever map filters change, so the tracks are recomputed. */
  filterKey: string;
  /** Service node IDs switched off by the user. */
  muted: string[];
  onMuted: (ids: string[]) => void;
  /** Time span of the focused request: the view zooms to it whenever it changes. */
  focus?: View | null;
  /** Keeps the zoom while the timeline is unmounted. */
  memory?: WorkspaceMemory;
}

function isoNs(iso: string | null | undefined): bigint | null {
  if (!iso) return null;
  const ms = Date.parse(iso);
  return Number.isFinite(ms) ? BigInt(ms) * NS_PER_MS : null;
}

function useTracks(actions: WorkspaceActions, view: View, buckets: number, clipCap: number, key: string, enabled: boolean) {
  const [tracks, setTracks] = useState<TimelineTracks | null>(null);
  const busy = useRef(false);
  useEffect(() => {
    if (!enabled || buckets <= 0) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      if (busy.current) return;
      busy.current = true;
      void actions
        .timelineTracks(view.from, view.to, buckets, clipCap)
        .then((t) => {
          if (!cancelled && t) setTracks(t);
        })
        .finally(() => {
          busy.current = false;
        });
    }, REFETCH_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [actions, view.from, view.to, buckets, clipCap, key, enabled]);
  return tracks;
}

function setupCanvas(canvas: HTMLCanvasElement | null, width: number, height: number) {
  if (!canvas || width <= 0 || height <= 0) return null;
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  canvas.width = Math.floor(width * dpr);
  canvas.height = Math.floor(height * dpr);
  const ctx = canvas.getContext("2d");
  ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
  return ctx;
}

type Drag =
  | { mode: "scrub" }
  | { mode: "pan"; x: number; view: View }
  /** On the tracks: a click places the cursor, a drag selects a replay range. */
  | { mode: "select"; x: number; t: bigint; moved: boolean }
  /** Moves one edge of the replay range; `fixed` is the other edge. */
  | { mode: "edge"; fixed: bigint };

interface Hover {
  x: number;
  y: number | null;
  clientX: number;
  clientY: number;
}

export function TimelineTracksView({ projectId, environmentId, state, actions, markA, filterKey, muted, onMuted, focus = null, memory }: Props) {
  /** What is loaded; replay ranges stay inside it. */
  const data = useMemo<View>(() => ({ from: state.rangeFrom, to: state.rangeTo }), [state.rangeFrom, state.rangeTo]);
  const [width, setWidth] = useState(0);
  const share = hasFuture(state.mode) ? futureShare(width) : 0;
  /** What can be shown: after live data, a short stretch that has not happened yet. */
  const bounds = useMemo<View>(() => withFuture(data, share), [data, share]);
  const future = share > 0 ? data.to : null;
  const [userView, setUserView] = useState<View | null>(() => memory?.timelineView() ?? null);
  useEffect(() => memory?.saveTimelineView(userView), [memory, userView]);
  const [hover, setHover] = useState<Hover | null>(null);
  const [dragging, setDragging] = useState(false);
  const [draft, setDraft] = useState<View | null>(null);
  const [prevLoop, setPrevLoop] = useState(state.loop);
  if (prevLoop !== state.loop) {
    // Clearing the replay range shows the whole timeline again (derived-state update during render).
    setPrevLoop(state.loop);
    if (!state.loop) setUserView(null);
  }
  const [prevFocus, setPrevFocus] = useState<View | null>(null);
  if (prevFocus !== focus) {
    setPrevFocus(focus);
    if (focus) setUserView(viewAround(focus, bounds));
  }
  const range = draft ?? state.loop;
  const hostRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const rulerRef = useRef<HTMLCanvasElement | null>(null);
  const rowsRef = useRef<HTMLCanvasElement | null>(null);

  let view = userView ? clampView(userView, bounds) : bounds;
  if (userView) {
    // Keep the playhead in view (derived-state update during render): live keeps the free
    // stretch on the right, replay pages along.
    const next =
      state.mode === "live"
        ? followLive(view, state.cursor, bounds, share)
        : state.mode === "replay" && (state.cursor < view.from || state.cursor > view.to)
          ? follow(view, state.cursor, bounds)
          : view;
    if (next !== view) {
      view = next;
      setUserView(next);
    }
  }
  const fitted = userView === null;
  const setView = (v: View) => setUserView(v.from === bounds.from && v.to === bounds.to ? null : v);

  const ready = state.phase === "ready";
  const dataKey = `${state.load?.at ?? 0}|${filterKey}`;
  const tracks = useTracks(actions, view, Math.floor(width / BUCKET_PX), CLIP_CAP, dataKey, ready);

  const plans = useQuery({
    queryKey: ["actions", projectId, environmentId],
    enabled: ready,
    queryFn: async () =>
      unwrap(
        await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/actions", {
          params: { path: scopedPath(projectId, environmentId) },
        }),
      ),
    refetchInterval: 15_000,
  });
  const actionBars = useMemo<ActionBar[]>(() => {
    const out: ActionBar[] = [];
    for (const p of plans.data ?? []) {
      const from = isoNs(p.started_at);
      if (from === null) continue;
      out.push({ id: p.id, title: p.title, target: p.target, state: p.state, from, to: isoNs(p.finished_at) ?? data.to });
    }
    return out;
  }, [plans.data, data.to]);

  const nodeIds = useMemo(() => {
    const ids = new Map<number, string>();
    for (const n of state.topology?.nodes ?? []) ids.set(n.index, n.id);
    return ids;
  }, [state.topology]);
  const serviceIds = useMemo(() => (state.topology?.nodes ?? []).filter((n) => n.kind === "service").map((n) => n.id), [state.topology]);
  const colors = useMemo(() => serviceColors(state.topology?.nodes ?? []), [state.topology]);
  const mutedSet = useMemo(() => new Set(muted), [muted]);

  const rows = useMemo<Row[]>(() => {
    const base: Row[] = [
      { id: "actions", label: "Actions", kind: "actions", height: 18 },
      { id: "events", label: "Events", kind: "events", height: 20 },
      { id: "errors", label: "Errors", kind: "errors", height: 20 },
      { id: "requests", label: "Requests", kind: "requests", height: 24 },
      { id: "latency", label: "Latency p95", kind: "latency", height: 26 },
    ];
    const services = tracks?.services ?? [];
    if (services.length === 0) return base;
    return base.concat(
      { id: "services", label: "Services", kind: "group", height: 22 },
      services.map((s, i) => {
        const nodeId = nodeIds.get(s.node);
        return {
          id: `svc-${s.node}`,
          label: s.name,
          kind: "service" as const,
          height: 26,
          track: i,
          nodeId,
          dimmed: nodeId !== undefined && mutedSet.has(nodeId),
          color: cssColor(colors.get(s.node) ?? RESOURCE_COLOR),
        };
      }),
    );
  }, [tracks?.services, nodeIds, mutedSet, colors]);
  const height = totalHeight(rows);

  const toggleService = (id: string, on: boolean) => onMuted(on ? muted.filter((m) => m !== id) : [...muted, id]);
  const soloService = (id: string) => onMuted(serviceIds.filter((s) => s !== id));

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const ro = new ResizeObserver(() => setWidth(Math.max(0, Math.floor(host.clientWidth))));
    ro.observe(host);
    return () => ro.disconnect();
  }, []);

  const hoverX = hover?.x ?? null;
  useEffect(() => {
    const ctx = setupCanvas(rulerRef.current, width, RULER_H);
    if (ctx) drawRuler(ctx, { width, view, cursor: state.cursor, markA, hoverX, range, future });
  }, [width, view, state.cursor, markA, hoverX, range, future]);

  useEffect(() => {
    const ctx = setupCanvas(rowsRef.current, width, height);
    if (ctx) draw(ctx, { width, rows, view, tracks, actions: actionBars, cursor: state.cursor, markA, hoverX, range, future });
  }, [width, height, rows, view, tracks, actionBars, state.cursor, markA, hoverX, range, future]);

  // Wheel and drag need the latest values without re-binding listeners.
  const live = useRef({ view, bounds, width, setView });
  useEffect(() => {
    live.current = { view, bounds, width, setView };
  });

  useEffect(() => {
    const scroller = scrollRef.current;
    const host = hostRef.current;
    if (!scroller || !host) return;
    const onWheel = (e: WheelEvent) => {
      const { view: v, bounds: b, width: w, setView: set } = live.current;
      const x = e.clientX - host.getBoundingClientRect().left;
      if (x < 0 || w <= 0) return;
      const overRuler = e.target === rulerRef.current;
      if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
        e.preventDefault();
        const d = e.shiftKey && e.deltaX === 0 ? e.deltaY : e.deltaX;
        set(panBy(v, BigInt(Math.round((d / w) * Number(v.to - v.from))), b));
        return;
      }
      // Plain wheel scrolls the rows; Ctrl/⌘ (and trackpad pinch) or the ruler zooms.
      if (!e.ctrlKey && !e.metaKey && !overRuler) return;
      e.preventDefault();
      set(zoomAt(v, timeAt(x, v, w), Math.exp(e.deltaY * (e.ctrlKey ? 0.01 : 0.002)), b));
    };
    scroller.addEventListener("wheel", onWheel, { passive: false });
    return () => scroller.removeEventListener("wheel", onWheel);
  }, []);

  const drag = useRef<Drag | null>(null);
  const pending = useRef<number | null>(null);
  const raf = useRef(0);
  const scrubTo = (clientX: number) => {
    const host = hostRef.current;
    if (!host || width <= 0) return;
    const x = Math.max(0, Math.min(width, clientX - host.getBoundingClientRect().left));
    pending.current = x;
    if (raf.current) return;
    raf.current = requestAnimationFrame(() => {
      raf.current = 0;
      const px = pending.current;
      const { view: v, width: w } = live.current;
      if (px !== null) actions.seek(timeAt(px, v, w));
    });
  };
  useEffect(() => () => cancelAnimationFrame(raf.current), []);

  const localX = (clientX: number) => {
    const host = hostRef.current;
    return host ? Math.max(0, Math.min(width, clientX - host.getBoundingClientRect().left)) : 0;
  };
  /** The range edge within reach of `x`, as the opposite (fixed) edge. */
  const edgeNear = (x: number): bigint | null => {
    const loop = state.loop;
    if (!loop || width <= 0) return null;
    if (Math.abs(x - xOf(loop.from, view, width)) <= EDGE_PX) return loop.to;
    if (Math.abs(x - xOf(loop.to, view, width)) <= EDGE_PX) return loop.from;
    return null;
  };

  // Shared by the ruler and the rows; `data-area="rows"` marks the tracks, where dragging
  // selects a replay range instead of moving the cursor.
  const onPointerDown = (e: React.PointerEvent<HTMLElement>) => {
    if (e.button !== 0 && e.button !== 1) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const onTracks = e.currentTarget.dataset.area === "rows";
    if (e.shiftKey || e.button === 1) {
      drag.current = { mode: "pan", x: e.clientX, view };
    } else if (onTracks) {
      // Keyboard (arrows, Esc) keeps working after a click on the tracks.
      hostRef.current?.focus({ preventScroll: true });
      const x = localX(e.clientX);
      const fixed = edgeNear(x);
      drag.current = fixed !== null ? { mode: "edge", fixed } : { mode: "select", x: e.clientX, t: timeAt(x, view, width), moved: false };
    } else {
      drag.current = { mode: "scrub" };
      scrubTo(e.clientX);
    }
    setDragging(true);
  };
  const onPointerMove = (e: React.PointerEvent<HTMLElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const d = drag.current;
    if (!d) {
      setHover({
        x: e.clientX - rect.left,
        y: e.currentTarget.dataset.area === "rows" ? e.clientY - rect.top : null,
        clientX: e.clientX,
        clientY: e.clientY,
      });
      return;
    }
    setHover(null);
    if (d.mode === "scrub") scrubTo(e.clientX);
    else if (d.mode === "select") {
      if (!d.moved && Math.abs(e.clientX - d.x) < DRAG_PX) return;
      d.moved = true;
      setDraft(rangeBetween(d.t, timeAt(localX(e.clientX), view, width), data));
    } else if (d.mode === "edge") setDraft(rangeBetween(d.fixed, timeAt(localX(e.clientX), view, width), data));
    else if (width > 0) setView(panBy(d.view, BigInt(Math.round((-(e.clientX - d.x) / width) * Number(d.view.to - d.view.from))), bounds));
  };
  const onPointerUp = (e: React.PointerEvent<HTMLElement>) => {
    const d = drag.current;
    drag.current = null;
    setDragging(false);
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    if (d?.mode === "select" && !d.moved) {
      actions.seek(timeAt(localX(e.clientX), view, width));
    } else if (d?.mode === "select" || d?.mode === "edge") {
      if (draft) {
        actions.setLoop(draft);
        if (d.mode === "select") setView(viewAround(draft, bounds));
      }
      setDraft(null);
    }
  };
  const onPointerLeave = () => {
    if (!drag.current) setHover(null);
  };
  const onPointerCancel = () => {
    drag.current = null;
    setDragging(false);
    setDraft(null);
  };
  const pointer = { onPointerDown, onPointerMove, onPointerUp, onPointerLeave, onPointerCancel };

  const zoomButton = (factor: number) => {
    const inView = state.cursor >= view.from && state.cursor <= view.to;
    setView(zoomCentered(view, inView ? state.cursor : view.from + (view.to - view.from) / 2n, factor, bounds));
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape" && state.loop) {
      e.preventDefault();
      actions.setLoop(null);
      return;
    }
    const span = view.to - view.from;
    const step = (span * BigInt(e.shiftKey ? 10 : 1)) / 100n;
    const keys: Record<string, () => bigint> = {
      ArrowLeft: () => state.cursor - step,
      ArrowRight: () => state.cursor + step,
      PageUp: () => state.cursor - span / 10n,
      PageDown: () => state.cursor + span / 10n,
      Home: () => data.from,
      End: () => data.to,
    };
    const next = keys[e.key];
    if (!next) return;
    e.preventDefault();
    actions.seek(next());
  };

  const percent = data.to > data.from ? Math.round(xOf(state.cursor, data, 100)) : 100;
  const futureX = future !== null && width > 0 ? xOf(future, view, width) : null;
  const tip =
    hover && hover.y !== null && !dragging
      ? future !== null && width > 0 && timeAt(hover.x, view, width) > future
        ? ["Future", "Nothing has happened here yet. Move the cursor here to go back to live."]
        : describe({ x: hover.x, y: hover.y }, rows, view, width, tracks, actionBars)
      : null;

  return (
    <div
      ref={scrollRef}
      className="relative min-h-0 flex-1 overflow-x-hidden overflow-y-auto rounded-lg border border-border bg-bg/50 select-none"
    >
      <div className="sticky top-0 z-20 flex border-b border-border bg-surface">
        <div className="flex shrink-0 items-center gap-0.5 border-r border-border px-1" style={{ width: LABEL_W, height: RULER_H }}>
          <button type="button" className="tl-btn" aria-label="Zoom out time" title="Zoom out (Ctrl+wheel)" onClick={() => zoomButton(2)}>
            <Icon name="minus" className="h-3 w-3" />
          </button>
          <button type="button" className="tl-btn" aria-label="Zoom in time" title="Zoom in around the cursor (Ctrl+wheel)" onClick={() => zoomButton(0.5)}>
            <Icon name="plus" className="h-3 w-3" />
          </button>
          <button
            type="button"
            className={`tl-btn ${fitted ? "text-accent" : ""}`}
            aria-label="Whole range"
            aria-pressed={fitted}
            title="Show the whole loaded range"
            onClick={() => setUserView(null)}
          >
            <Icon name="maximize" className="h-3 w-3" />
          </button>
          <span className="ml-auto truncate pr-1 font-mono text-[10px] text-muted" title="Visible time span">
            {formatSpan(view.to - view.from)}
          </span>
        </div>
        <div
          ref={hostRef}
          role="slider"
          tabIndex={0}
          aria-label="Selected time"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.max(0, Math.min(100, percent))}
          aria-valuetext={formatDateTime(state.cursor)}
          onKeyDown={onKeyDown}
          className="relative min-w-0 flex-1 cursor-ew-resize touch-none outline-none focus-visible:ring-1 focus-visible:ring-accent"
          style={{ height: RULER_H }}
          title="Drag to move through time · Wheel: zoom"
          {...pointer}
        >
          <canvas ref={rulerRef} aria-hidden className="absolute inset-0" style={{ width, height: RULER_H }} />
          {futureX !== null && futureX < width - 40 ? (
            <span
              className="pointer-events-none absolute inset-y-0 right-0 flex items-center justify-center text-[10px] font-semibold tracking-[0.18em] text-text/70"
              style={{ left: Math.max(0, futureX) }}
            >
              FUTURE
            </span>
          ) : null}
        </div>
      </div>

      <div className="flex">
        <ul aria-label="Tracks" className="shrink-0 border-r border-border text-[11px]" style={{ width: LABEL_W }}>
          {rows.map((r) => {
            if (r.kind === "group") {
              const allOn = muted.length === 0;
              const allOff = serviceIds.length > 0 && serviceIds.every((id) => mutedSet.has(id));
              return (
                <li key={r.id} className="flex items-center gap-1 border-b border-border px-2.5 text-[10px] font-semibold tracking-wider text-muted uppercase" style={{ height: r.height }}>
                  {r.label}
                  <button type="button" className="ml-auto rounded px-1 py-px normal-case hover:bg-surface-3 hover:text-text disabled:opacity-40" disabled={allOn} onClick={() => onMuted([])}>
                    All on
                  </button>
                  <button type="button" className="rounded px-1 py-px normal-case hover:bg-surface-3 hover:text-text disabled:opacity-40" disabled={allOff} onClick={() => onMuted(serviceIds)}>
                    All off
                  </button>
                </li>
              );
            }
            if (r.kind === "service" && r.nodeId !== undefined) {
              const id = r.nodeId;
              return (
                <li key={r.id} className="group flex items-center gap-1.5 px-2.5" style={{ height: r.height }} title={r.label}>
                  <input
                    type="checkbox"
                    className="switch"
                    checked={!r.dimmed}
                    onChange={(e) => toggleService(id, e.target.checked)}
                    aria-label={`Show ${r.label}`}
                    title={r.dimmed ? "Switched off: dimmed here and on the map" : "Switch off to dim it here and on the map"}
                  />
                  <span
                    className={`dot shrink-0 ${r.dimmed ? "opacity-40" : ""}`}
                    style={{ background: r.color }}
                    data-service-color={r.color}
                    aria-hidden
                  />
                  <span className={`truncate ${r.dimmed ? "text-muted/60" : ""}`}>{r.label}</span>
                  <button
                    type="button"
                    className="tl-btn ml-auto opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                    onClick={() => soloService(id)}
                    aria-label={`Show only ${r.label}`}
                    title="Show only this service"
                  >
                    <Icon name="crosshair" className="h-3 w-3" />
                  </button>
                  {tracks && r.track !== undefined ? (
                    <span className={`w-6 text-right font-mono text-[10px] ${r.dimmed ? "text-muted/50" : "text-muted"}`}>
                      {tracks.services[r.track]?.spans_in_view ?? 0}
                    </span>
                  ) : null}
                </li>
              );
            }
            return (
              <li key={r.id} className="flex items-center gap-1.5 truncate px-2.5" style={{ height: r.height }} title={r.label}>
                <span className={`dot ${ROW_DOT[r.kind]}`} aria-hidden />
                <span className="truncate">{r.label}</span>
              </li>
            );
          })}
        </ul>
        <div
          className={`relative min-w-0 flex-1 touch-none ${hover && edgeNear(hover.x) !== null ? "cursor-col-resize" : "cursor-crosshair"}`}
          style={{ height }}
          data-area="rows"
          {...pointer}
        >
          <canvas ref={rowsRef} aria-hidden className="absolute inset-0" style={{ width, height }} />
        </div>
        {tip && hover ? <CursorTooltip x={hover.clientX} y={hover.clientY} lines={tip} /> : null}
      </div>
    </div>
  );
}

const ROW_DOT: Record<Row["kind"], string> = {
  actions: "bg-ok",
  events: "bg-accent-2",
  errors: "bg-error",
  requests: "bg-accent",
  latency: "bg-violet",
  group: "bg-transparent",
  service: "bg-accent/60",
};

/**
 * Tooltip centred right above the pointer, kept inside the window (flipped below near the top).
 * Rendered into `document.body` so no scrolling or clipping ancestor can cut it off.
 */
function CursorTooltip({ x, y, lines }: { x: number; y: number; lines: string[] }) {
  const ref = useRef<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const p = placeAbove(x, y, el.offsetWidth, el.offsetHeight, window.innerWidth, window.innerHeight);
    el.style.left = `${p.left}px`;
    el.style.top = `${p.top}px`;
    el.style.visibility = "visible";
  });
  return createPortal(
    <div
      ref={ref}
      role="tooltip"
      className="pointer-events-none fixed top-0 left-0 z-50 max-w-72 rounded-lg border border-border-strong bg-surface-2/95 px-2.5 py-1.5 text-[11px] shadow-xl backdrop-blur"
      style={{ visibility: "hidden" }}
    >
      {lines.map((line, i) => (
        <div key={i} className={i === 0 ? "font-mono text-muted" : ""}>
          {line}
        </div>
      ))}
    </div>,
    document.body,
  );
}

/** Tooltip lines for the item under the pointer; the first line is the time. */
function describe(
  hover: { x: number; y: number },
  rows: Row[],
  view: View,
  width: number,
  tracks: TimelineTracks | null,
  bars: ActionBar[],
): string[] | null {
  if (width <= 0) return null;
  const index = rows.findIndex((_, i) => hover.y >= rowTop(rows, i) && hover.y < rowTop(rows, i + 1));
  const row = rows[index];
  if (!row) return null;
  const t = timeAt(hover.x, view, width);
  const fine = view.to - view.from < 2_000_000_000n;
  const clock = (ns: bigint) => (fine ? formatClockMicros(ns) : formatClock(ns));
  if (row.kind === "actions") {
    const a = bars.find((b) => t >= b.from && t <= b.to);
    return a ? [clock(t), `${a.title} · ${a.target}`, a.state] : null;
  }
  if (!tracks) return null;
  const g = bucketGeometry(tracks, view, width);
  const i = Math.floor(Number(t - g.from) / g.bucketNs);
  if (i < 0 || i >= tracks.buckets) return null;
  const b0 = g.from + BigInt(Math.round(i * g.bucketNs));
  const b1 = g.from + BigInt(Math.round((i + 1) * g.bucketNs));
  const range = `${clock(b0)} – ${clock(b1)}`;
  switch (row.kind) {
    case "events": {
      const near = tracks.markers.filter((m) => Math.abs(g.x0 + m.time_ms * 1e6 * g.pxPerNs - hover.x) <= 5);
      return [range, `${tracks.events[i] ?? 0} events`, ...near.slice(0, 4).map((m) => `◆ ${m.event_type}${m.service_name ? ` · ${m.service_name}` : ""}`)];
    }
    case "errors":
      return [range, `${tracks.errors[i] ?? 0} errors (failed requests + error logs)`];
    case "requests":
      return [range, `${tracks.requests[i] ?? 0} requests started`];
    case "latency": {
      const p = tracks.latency_p95_ms[i];
      const n = tracks.latency_samples[i] ?? 0;
      return [range, p === null || p === undefined ? "no request ended here" : `p95 ${p.toFixed(p < 10 ? 2 : 1)} ms · ${n} requests`];
    }
    case "service": {
      const s = row.track !== undefined ? tracks.services[row.track] : undefined;
      if (!s) return null;
      if (s.clipped) {
        const ms = Number(t - g.from) / 1e6;
        const hit = tracks.clips
          .filter((c) => c.track === row.track && ms >= c.start_ms && ms <= c.end_ms)
          .sort((a, b) => b.depth - a.depth)[0];
        if (!hit) return [clock(t), `${s.name}: idle`];
        const dur = hit.end_ms - hit.start_ms;
        return [
          `${clock(g.from + BigInt(Math.round(hit.start_ms * 1e6)))} +${dur < 1 ? `${(dur * 1000).toFixed(0)} µs` : `${dur.toFixed(2)} ms`}`,
          `${s.name} · ${hit.name}`,
          hit.error ? "error" : "ok",
        ];
      }
      return [range, `${s.counts[i] ?? 0} spans started in ${s.name}`, `zoom in to see individual spans (${s.spans_in_view} in view)`];
    }
    default:
      return null;
  }
}
