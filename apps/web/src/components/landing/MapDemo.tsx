"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { nodeSubLabel } from "@/renderer/external";
import { ACCENT, CARD_BG, EDGE_ERROR, EDGE_INFERRED, EDGE_OK, EDGE_WARN, EXTERNAL_COLOR, STATUS_COLORS, hex } from "@/renderer/map-colors";
import {
  BUCKET,
  EDGES,
  type Health,
  MARKERS,
  NODES,
  type Orientation,
  VIEW,
  WINDOW,
  curvePath,
  edgeCurve,
  pointOnCurve,
  relativeLabel,
  serviceColor,
  stateAt,
  timelineBuckets,
} from "./demo-model";

const PLAY_SPEED = 30;
const STATUS: Record<Health, number> = { ok: 1, degraded: 2, failing: 3 };
const HEALTH_TEXT: Record<Health, string> = { ok: "healthy", degraded: "degraded", failing: "failing" };
const TICKS = ["−15:00", "−10:00", "−5:00", "now"];

function useMedia(query: string): boolean {
  const [matches, setMatches] = useState(false);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const update = () => setMatches(mq.matches);
    update();
    mq.addEventListener("change", update);
    return () => mq.removeEventListener("change", update);
  }, [query]);
  return matches;
}

function statusColor(h: Health): string {
  return hex(STATUS_COLORS[STATUS[h]] ?? STATUS_COLORS[0]);
}

/** The workspace's system map and timeline, running on a synthetic system. */
export function MapDemo() {
  const [t, setT] = useState(WINDOW);
  const [playing, setPlaying] = useState(false);
  const [clock, setClock] = useState(WINDOW);
  const [selected, setSelected] = useState("orders");
  const [visible, setVisible] = useState(true);
  const reduced = useMedia("(prefers-reduced-motion: reduce)");
  const narrow = useMedia("(max-width: 639px)");
  const o: Orientation = narrow ? "narrow" : "wide";
  const view = VIEW[o];
  const root = useRef<HTMLDivElement>(null);
  const live = t >= WINDOW;

  useEffect(() => {
    const el = root.current;
    if (!el || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver(([entry]) => setVisible(entry?.isIntersecting ?? true));
    io.observe(el);
    return () => io.disconnect();
  }, []);

  useEffect(() => {
    if (!visible) return;
    if (live && reduced) return;
    if (!live && !playing) return;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      if (live) {
        setClock((c) => c + dt);
      } else {
        setT((prev) => {
          const next = Math.min(WINDOW, prev + dt * PLAY_SPEED);
          setClock(next);
          if (next >= WINDOW) setPlaying(false);
          return next;
        });
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [live, playing, reduced, visible]);

  const state = useMemo(() => stateAt(t, live ? clock : t), [t, clock, live]);
  const curves = useMemo(() => EDGES.map((e) => edgeCurve(e, o)), [o]);
  const node = NODES.find((n) => n.id === selected) ?? NODES[0];
  const selectedHealth: Health = (node && state.health[node.id]) || "ok";
  const nearMarker = MARKERS.find((m) => Math.abs(m.t - t) < 45);
  const failing = NODES.filter((n) => state.health[n.id] === "failing").map((n) => n.label);
  const degraded = NODES.filter((n) => state.health[n.id] === "degraded").map((n) => n.label);
  const summary =
    `Service map at ${relativeLabel(t)}. ` +
    (failing.length ? `Failing: ${failing.join(", ")}. ` : "") +
    (degraded.length ? `Degraded: ${degraded.join(", ")}. ` : "") +
    (!failing.length && !degraded.length ? "All services healthy." : "");

  function scrub(value: number) {
    setPlaying(false);
    setT(value);
    setClock(value);
  }

  return (
    <div ref={root} className="overflow-hidden rounded-lg border border-border bg-bg-2" aria-label="Interactive demo" role="group">
      <div className="flex items-center gap-3 border-b border-border px-4 py-2.5 text-xs">
        <span className="flex items-center gap-2 font-semibold text-text">
          <Icon name="layers" className="h-3.5 w-3.5 text-accent" />
          System Map
        </span>
        <span className="hidden text-muted sm:inline">checkout · production</span>
        <span className="ml-auto font-mono text-[11px] text-muted">Demo · synthetic data</span>
      </div>

      <svg viewBox={`0 0 ${view.w} ${view.h}`} className="block h-auto w-full bg-bg" role="img" aria-label={summary}>
        <defs>
          <pattern id="demo-grid" width="24" height="24" patternUnits="userSpaceOnUse">
            <path d="M24 0H0V24" fill="none" stroke="#2a2d31" strokeWidth="1" />
          </pattern>
        </defs>
        <rect width={view.w} height={view.h} fill="url(#demo-grid)" />

        {EDGES.map((edge, i) => {
          const c = curves[i];
          if (!c) return null;
          const hFrom = state.health[edge.from] ?? "ok";
          const hTo = state.health[edge.to] ?? "ok";
          const problem = hFrom === "failing" || hTo === "failing" ? EDGE_ERROR : hTo === "degraded" ? EDGE_WARN : null;
          const external = NODES.find((n) => n.id === edge.from)?.external || NODES.find((n) => n.id === edge.to)?.external;
          const color = hex(problem ?? (external ? EXTERNAL_COLOR : edge.kind === "inferred" ? EDGE_INFERRED : EDGE_OK));
          const d = curvePath(c);
          const [ex, ey] = c[3];
          const head = o === "wide" ? `M${ex} ${ey - 4}L${ex + 6} ${ey}L${ex} ${ey + 4}Z` : `M${ex - 4} ${ey}L${ex} ${ey + 6}L${ex + 4} ${ey}Z`;
          return (
            <g key={`${edge.from}-${edge.to}`}>
              {edge.kind === "observed" ? <path d={d} fill="none" stroke={color} strokeWidth={8} strokeOpacity={0.1} strokeLinecap="round" /> : null}
              <path
                d={d}
                fill="none"
                stroke={color}
                strokeWidth={2}
                strokeOpacity={edge.kind === "inferred" ? 0.8 : 0.9}
                strokeDasharray={edge.kind === "inferred" ? "6 5" : undefined}
                strokeLinecap="round"
              />
              <path d={head} fill={color} />
              {edge.kind === "inferred" ? <title>{edge.reason}</title> : null}
            </g>
          );
        })}

        {state.dots.map((dot, i) => {
          const c = curves[dot.edge];
          if (!c) return null;
          const [x, y] = pointOnCurve(c, dot.at);
          return (
            <circle
              key={`${dot.edge}-${i}`}
              cx={x}
              cy={y}
              r={3.5}
              fill={dot.error ? hex(CARD_BG) : hex(ACCENT)}
              stroke={dot.error ? hex(EDGE_ERROR) : "none"}
              strokeWidth={dot.error ? 2 : 0}
            />
          );
        })}

        {NODES.map((n) => {
          const h = state.health[n.id] ?? "ok";
          const status = STATUS[h];
          const own = serviceColor(n.id);
          const color = hex(status >= 2 ? (STATUS_COLORS[status] ?? EDGE_ERROR) : n.external ? EXTERNAL_COLOR : (own ?? EXTERNAL_COLOR));
          const [cx, cy] = n[o];
          const { w, h: ch } = view.card;
          const left = cx - w / 2;
          const top = cy - ch / 2;
          const isSel = n.id === selected;
          const textX = narrow ? left + 10 : left + 46;
          const name = narrow && n.id === "clients" ? "clients" : n.label;
          const sub = n.external && !narrow ? "external" : nodeSubLabel(false, 0, state.requests[n.id] ?? 0);
          return (
            <g
              key={n.id}
              role="button"
              tabIndex={0}
              aria-label={`${n.label}, ${HEALTH_TEXT[h]}`}
              aria-pressed={isSel}
              className="group cursor-pointer outline-none"
              onClick={() => setSelected(n.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  setSelected(n.id);
                }
              }}
            >
              <rect
                x={left - 6}
                y={top - 6}
                width={w + 12}
                height={ch + 12}
                rx={16}
                fill="none"
                stroke={isSel ? "#ffffff" : hex(ACCENT)}
                strokeOpacity={isSel ? 0.9 : 0}
                strokeWidth={2}
                className="group-focus-visible:[stroke-opacity:1]"
              />
              {n.external ? (
                <rect x={left} y={top} width={w} height={ch} rx={4} fill={hex(CARD_BG)} fillOpacity={0.9} stroke={color} strokeWidth={1.75} strokeDasharray="7 5" />
              ) : (
                <>
                  <rect x={left - 3} y={top - 3} width={w + 6} height={ch + 6} rx={15} fill="none" stroke={color} strokeWidth={6} strokeOpacity={status >= 2 ? 0.22 : n.inferred ? 0 : 0.1} />
                  <rect x={left} y={top} width={w} height={ch} rx={12} fill={hex(CARD_BG)} stroke={color} strokeWidth={1.5} strokeOpacity={n.inferred ? 0.45 : 0.85} />
                </>
              )}
              {narrow ? null : (
                <>
                  <rect x={left + 10} y={cy - 14} width={28} height={28} rx={8} fill={color} fillOpacity={0.14} />
                  {n.external ? (
                    <g fill="none" stroke={color} strokeWidth={1.5}>
                      <circle cx={left + 24} cy={cy} r={6} />
                      <path d={`M${left + 18} ${cy}h12M${left + 24} ${cy - 6}c3 3 3 9 0 12c-3-3-3-9 0-12`} />
                    </g>
                  ) : (
                    <g fill="none" stroke={color} strokeWidth={1.5}>
                      <rect x={left + 18} y={cy - 6} width={12} height={5} rx={1} />
                      <rect x={left + 18} y={cy + 1} width={12} height={5} rx={1} />
                    </g>
                  )}
                </>
              )}
              <circle cx={left + w - 11} cy={top + 11} r={3.5} fill={statusColor(h)} />
              <text x={textX} y={cy - 3} fontSize={narrow ? 13 : 13.5} className="fill-text font-semibold" style={{ fontFamily: "var(--font-sans)" }}>
                {name}
              </text>
              <text x={textX} y={cy + 13} fontSize={10.5} className="fill-muted" style={{ fontFamily: "var(--font-mono)" }}>
                {sub}
              </text>
            </g>
          );
        })}
      </svg>

      <MapLegend />

      <Timeline t={t} live={live} playing={playing} onPlay={() => setPlaying((p) => !p)} onScrub={scrub} />

      <div className="border-t border-border px-4 py-3">
        <dl className="grid grid-cols-2 gap-x-6 gap-y-1 text-xs sm:grid-cols-4">
          <div className="flex items-baseline gap-2">
            <dt className="text-muted">Service</dt>
            <dd className="truncate font-semibold">{node?.label}</dd>
          </div>
          <div className="flex items-baseline gap-2">
            <dt className="text-muted">State</dt>
            <dd className="font-medium" style={{ color: statusColor(selectedHealth) }}>
              {HEALTH_TEXT[selectedHealth]}
            </dd>
          </div>
          <div className="flex items-baseline gap-2">
            <dt className="text-muted">p95</dt>
            <dd className="font-mono tabular-nums">{node ? state.p95ms[node.id] : 0} ms</dd>
          </div>
          <div className="flex items-baseline gap-2">
            <dt className="text-muted">Errors</dt>
            <dd className="font-mono tabular-nums">{Math.round((node ? (state.errorRate[node.id] ?? 0) : 0) * 100)}%</dd>
          </div>
        </dl>
        <p className="mt-2 min-h-4 text-xs text-muted">{nearMarker ? nearMarker.label : "Drag the timeline back to see the system before the fault."}</p>
      </div>
    </div>
  );
}

function MapLegend() {
  const status: Array<{ label: string; color: number }> = [
    { label: "Healthy", color: STATUS_COLORS[1] },
    { label: "Degraded", color: STATUS_COLORS[2] },
    { label: "Failing", color: STATUS_COLORS[3] },
  ];
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-border px-4 py-2 text-[11px] text-muted" aria-hidden>
      {status.map((s) => (
        <span key={s.label} className="flex items-center gap-1.5">
          <span className="dot" style={{ background: hex(s.color) }} />
          {s.label}
        </span>
      ))}
      <span className="h-3 w-px bg-border-strong" />
      <span className="flex items-center gap-1.5">
        <span className="inline-block h-0.5 w-4" style={{ background: hex(EDGE_OK) }} /> Observed
      </span>
      <span className="flex items-center gap-1.5">
        <span className="inline-block w-4 border-t-2 border-dashed" style={{ borderColor: hex(EDGE_INFERRED) }} /> Inferred
      </span>
      <span className="flex items-center gap-1.5">
        <span className="inline-block h-2 w-2 rounded-full" style={{ background: hex(ACCENT) }} /> Request
      </span>
    </div>
  );
}

const ROWS = [
  { id: "actions", label: "Actions", color: "#22c55e", h: 16 },
  { id: "events", label: "Events", color: "#9a9fa6", h: 18 },
  { id: "errors", label: "Errors", color: "#ef4444", h: 18 },
  { id: "requests", label: "Requests", color: "#3b82f6", h: 22 },
  { id: "latency", label: "Latency p95", color: "#a855f7", h: 24 },
] as const;

function Timeline({ t, live, playing, onPlay, onScrub }: { t: number; live: boolean; playing: boolean; onPlay: () => void; onScrub: (v: number) => void }) {
  const buckets = useMemo(() => timelineBuckets(), []);
  const maxReq = Math.max(...buckets.map((b) => b.requests));
  const maxErr = Math.max(1, ...buckets.map((b) => b.errors));
  const maxLat = Math.max(...buckets.map((b) => b.latency));
  const n = buckets.length;
  const total = ROWS.reduce((s, r) => s + r.h, 0);
  const rowTop = (i: number) => ROWS.slice(0, i).reduce((s, r) => s + r.h, 0);
  const latencyPath = buckets
    .map((b, i) => {
      const r = ROWS[4];
      const y = rowTop(4) + r.h - 3 - (b.latency / maxLat) * (r.h - 6);
      return `${i === 0 ? "M" : "L"}${i + 0.5} ${y.toFixed(2)}`;
    })
    .join(" ");
  const pct = (t / WINDOW) * 100;

  return (
    <div className="border-t border-border">
      <div className="flex items-center gap-2 px-4 py-2">
        <span className={`badge ${live ? "border-ok/50 text-ok" : "border-border-strong text-muted"}`}>{live ? "LIVE" : "REPLAY"}</span>
        <button type="button" className="btn btn-icon btn-sm" aria-label={playing ? "Pause replay" : "Play replay"} disabled={live} onClick={onPlay}>
          <Icon name={playing ? "pause" : "play"} className="h-3.5 w-3.5" />
        </button>
        <span className="rounded-md border border-border bg-bg px-2 py-1 font-mono text-xs text-text tabular-nums" aria-live="polite">
          {relativeLabel(t)}
        </span>
        <button type="button" className={`btn btn-sm ml-auto ${live ? "border-ok/50 text-ok" : ""}`} aria-pressed={live} onClick={() => onScrub(WINDOW)}>
          <span className={`dot ${live ? "bg-ok" : "bg-muted"}`} aria-hidden />
          Live
        </button>
      </div>
      <div className="grid grid-cols-[6.75rem_minmax(0,1fr)] border-t border-border text-[11px]">
        <div className="pt-5" aria-hidden>
          {ROWS.map((r) => (
            <div key={r.id} className="flex items-center gap-1.5 px-3 text-muted" style={{ height: r.h }}>
              <span className="dot" style={{ background: r.color }} />
              <span className="truncate">{r.label}</span>
            </div>
          ))}
        </div>
        <div className="relative pr-4">
          <div className="pointer-events-none relative h-5 font-mono text-[10px] text-muted tabular-nums" aria-hidden>
            {TICKS.map((label, i) => (
              <span
                key={label}
                className={`absolute top-1 -translate-x-1/2 first:translate-x-0 last:-translate-x-full ${i === 1 ? "max-sm:hidden" : ""}`}
                style={{ left: `${(i / 3) * 100}%` }}
              >
                {label}
              </span>
            ))}
          </div>
          <div className="relative" style={{ height: total }}>
            <svg viewBox={`0 0 ${n} ${total}`} preserveAspectRatio="none" className="absolute inset-0 h-full w-full" aria-hidden>
              {ROWS.map((r, i) => (i % 2 === 1 ? <rect key={r.id} x={0} y={rowTop(i)} width={n} height={r.h} fill="rgba(39, 41, 45, 0.5)" /> : null))}
              {MARKERS.map((m) => (
                <rect key={m.t} x={m.t / BUCKET - 0.25} y={rowTop(m.kind === "action" ? 0 : 1) + 3} width={0.5} height={(m.kind === "action" ? ROWS[0].h : ROWS[1].h) - 6} fill={m.kind === "action" ? ROWS[0].color : "#eceef0"} />
              ))}
              {buckets.map((b, i) =>
                b.events > 0 ? <rect key={`e${i}`} x={i + 0.2} y={rowTop(1) + ROWS[1].h - 3 - b.events * 3} width={0.6} height={b.events * 3} fill={ROWS[1].color} fillOpacity={0.7} /> : null,
              )}
              {buckets.map((b, i) =>
                b.errors > 0 ? (
                  <rect key={`x${i}`} x={i + 0.15} y={rowTop(2) + ROWS[2].h - 2 - (b.errors / maxErr) * (ROWS[2].h - 5)} width={0.7} height={(b.errors / maxErr) * (ROWS[2].h - 5)} fill={ROWS[2].color} />
                ) : null,
              )}
              {buckets.map((b, i) => (
                <rect key={`r${i}`} x={i + 0.15} y={rowTop(3) + ROWS[3].h - 2 - (b.requests / maxReq) * (ROWS[3].h - 5)} width={0.7} height={(b.requests / maxReq) * (ROWS[3].h - 5)} fill={ROWS[3].color} fillOpacity={0.85} />
              ))}
              <path d={latencyPath} fill="none" stroke={ROWS[4].color} strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
            </svg>
            <input
              type="range"
              min={0}
              max={WINDOW}
              step={1}
              value={Math.round(t)}
              onChange={(e) => onScrub(Number(e.target.value))}
              aria-label="Replay time"
              aria-valuetext={relativeLabel(t)}
              className="peer absolute inset-0 h-full w-full cursor-ew-resize appearance-none opacity-0"
            />
            <span aria-hidden className="pointer-events-none absolute inset-0 rounded-sm peer-focus-visible:ring-2 peer-focus-visible:ring-accent" />
            <span aria-hidden className="pointer-events-none absolute -top-5 bottom-0 w-0.5 -translate-x-1/2 bg-accent-2" style={{ left: `${pct}%` }}>
              <span className="absolute -top-0.5 left-1/2 h-2.5 w-2.5 -translate-x-1/2 rounded-full bg-accent-2" />
            </span>
          </div>
          <div className="h-3" />
        </div>
      </div>
    </div>
  );
}
