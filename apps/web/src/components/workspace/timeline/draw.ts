// 2D-canvas drawing of the layered timeline. Pure: everything it needs is passed in.

import type { TimelineTracks } from "@controlhub/web-engine";
import { formatTick, tickStep, ticks, xOf, type View } from "./scale";

export type RowKind = "actions" | "events" | "errors" | "requests" | "latency" | "group" | "service";

export interface Row {
  id: string;
  label: string;
  kind: RowKind;
  height: number;
  /** Index into `TimelineTracks.services` for service rows. */
  track?: number;
  /** Topology node ID of a service row. */
  nodeId?: string;
  /** Switched off by the user: drawn faded. */
  dimmed?: boolean;
  /** `#rrggbb` of a service row, the same colour the service has on the map. */
  color?: string;
}

export interface ActionBar {
  id: string;
  title: string;
  target: string;
  state: string;
  from: bigint;
  to: bigint;
}

export interface DrawInput {
  width: number;
  rows: Row[];
  view: View;
  tracks: TimelineTracks | null;
  actions: ActionBar[];
  cursor: bigint;
  markA: bigint | null;
  hoverX: number | null;
  /** Replay range (or the one being dragged out). */
  range: View | null;
}

export interface RulerInput {
  width: number;
  view: View;
  cursor: bigint;
  markA: bigint | null;
  hoverX: number | null;
  range: View | null;
}

const DIMMED_ROW_ALPHA = 0.28;

/** Shades the time outside the replay range and marks its edges. */
function rangeBand(ctx: CanvasRenderingContext2D, range: View | null, view: View, width: number, height: number, handles: boolean) {
  if (!range) return;
  const x1 = Math.max(-2, Math.min(width + 2, xOf(range.from, view, width)));
  const x2 = Math.max(-2, Math.min(width + 2, xOf(range.to, view, width)));
  ctx.fillStyle = "rgba(6, 10, 19, 0.55)";
  if (x1 > 0) ctx.fillRect(0, 0, x1, height);
  if (x2 < width) ctx.fillRect(x2, 0, width - x2, height);
  ctx.fillStyle = "rgba(34, 211, 238, 0.06)";
  ctx.fillRect(x1, 0, x2 - x1, height);
  ctx.fillStyle = "rgba(34, 211, 238, 0.8)";
  for (const x of [x1, x2]) {
    if (x < 0 || x > width) continue;
    ctx.fillRect(Math.round(x) - 0.5, 0, 1.5, height);
    if (handles) roundRect(ctx, Math.round(x) - 2.5, height / 2 - 7, 5, 14, 2);
    if (handles) ctx.fill();
  }
}

/** Height of the ruler, drawn on its own canvas so it stays put while the rows scroll. */
export const RULER_H = 24;

const C = {
  grid: "rgba(38, 53, 90, 0.55)",
  rowAlt: "rgba(16, 26, 46, 0.45)",
  text: "#e6ecf5",
  muted: "#7d8aa5",
  accent: "#3b82f6",
  accent2: "#22d3ee",
  violet: "#a855f7",
  warn: "#f59e0b",
  error: "#ef4444",
  ok: "#22c55e",
};

/** `rgba(...)` of a `#rrggbb` colour. */
export function withAlpha(hex: string, a: number): string {
  const n = Number.parseInt(hex.replace("#", ""), 16);
  const v = Number.isFinite(n) ? n : 0;
  return `rgba(${(v >> 16) & 255}, ${(v >> 8) & 255}, ${v & 255}, ${a})`;
}

export function actionColor(state: string): string {
  if (state === "succeeded") return C.ok;
  if (state === "failed" || state === "verification_failed") return C.error;
  if (state === "running" || state === "approved") return C.warn;
  return C.muted;
}

export function rowTop(rows: Row[], index: number): number {
  let y = 0;
  for (let i = 0; i < index; i++) y += rows[i]?.height ?? 0;
  return y;
}

export function totalHeight(rows: Row[]): number {
  return rows.reduce((h, r) => h + r.height, 0);
}

/** Geometry of the tracks' buckets in the current view. */
export function bucketGeometry(t: TimelineTracks, view: View, width: number) {
  const from = BigInt(t.from_ns);
  const to = BigInt(t.to_ns);
  const bucketNs = Number(to - from) / Math.max(1, t.buckets);
  const pxPerNs = width / Math.max(1, Number(view.to - view.from));
  return { from, x0: xOf(from, view, width), bucketNs, pxPerNs, bucketPx: bucketNs * pxPerNs };
}

function bars(ctx: CanvasRenderingContext2D, values: number[], y: number, h: number, g: ReturnType<typeof bucketGeometry>, width: number, color: string) {
  let max = 0;
  for (const v of values) if (v > max) max = v;
  if (max <= 0) return;
  ctx.fillStyle = color;
  const w = Math.max(1, g.bucketPx - (g.bucketPx > 3 ? 1 : 0));
  for (let i = 0; i < values.length; i++) {
    const v = values[i] ?? 0;
    if (v <= 0) continue;
    const x = g.x0 + i * g.bucketPx;
    if (x + w < 0 || x > width) continue;
    const bh = Math.max(1.5, (v / max) * (h - 5));
    ctx.fillRect(x, y + h - 2 - bh, w, bh);
  }
}

function latency(ctx: CanvasRenderingContext2D, values: Array<number | null>, y: number, h: number, g: ReturnType<typeof bucketGeometry>) {
  let max = 0;
  for (const v of values) if (v !== null && v > max) max = v;
  if (max <= 0) return;
  const base = y + h - 2;
  const pt = (i: number, v: number) => [g.x0 + (i + 0.5) * g.bucketPx, base - (v / max) * (h - 6)] as const;
  let run: Array<readonly [number, number]> = [];
  const flush = () => {
    if (run.length === 0) return;
    const first = run[0];
    const last = run[run.length - 1];
    if (!first || !last) return;
    ctx.beginPath();
    ctx.moveTo(first[0], base);
    for (const [x, yy] of run) ctx.lineTo(x, yy);
    ctx.lineTo(last[0], base);
    ctx.closePath();
    ctx.fillStyle = "rgba(168, 85, 247, 0.22)";
    ctx.fill();
    ctx.beginPath();
    run.forEach(([x, yy], k) => (k === 0 ? ctx.moveTo(x, yy) : ctx.lineTo(x, yy)));
    ctx.strokeStyle = C.violet;
    ctx.lineWidth = 1.5;
    ctx.stroke();
    if (run.length === 1) {
      ctx.fillStyle = C.violet;
      ctx.fillRect(first[0] - 1.5, first[1] - 1.5, 3, 3);
    }
    run = [];
  };
  values.forEach((v, i) => {
    if (v === null) flush();
    else run.push(pt(i, v));
  });
  flush();
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  const rr = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + rr, y);
  ctx.arcTo(x + w, y, x + w, y + h, rr);
  ctx.arcTo(x + w, y + h, x, y + h, rr);
  ctx.arcTo(x, y + h, x, y, rr);
  ctx.arcTo(x, y, x + w, y, rr);
  ctx.closePath();
}

function clips(
  ctx: CanvasRenderingContext2D,
  t: TimelineTracks,
  track: number,
  y: number,
  h: number,
  g: ReturnType<typeof bucketGeometry>,
  width: number,
  base: string,
) {
  const list = t.clips.filter((c) => c.track === track);
  const depth = Math.min(3, list.reduce((d, c) => Math.max(d, c.depth), 0));
  const lane = (h - 4) / (depth + 1);
  ctx.font = "10px Inter, 'Segoe UI', sans-serif";
  ctx.textBaseline = "middle";
  for (const c of list) {
    const x = g.x0 + c.start_ms * 1e6 * g.pxPerNs;
    const w = Math.max(1.5, (c.end_ms - c.start_ms) * 1e6 * g.pxPerNs);
    if (x + w < 0 || x > width) continue;
    const yy = y + 2 + Math.min(c.depth, depth) * lane;
    const color = c.error ? C.error : base;
    roundRect(ctx, x, yy + 0.5, w, lane - 1, 2.5);
    ctx.fillStyle = withAlpha(color, c.depth === 0 ? 0.35 : 0.2);
    ctx.fill();
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.stroke();
    if (w > 46 && lane >= 9) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(Math.max(x, 0), yy, Math.min(w, width) - 2, lane);
      ctx.clip();
      ctx.fillStyle = C.text;
      ctx.fillText(c.name, Math.max(x, 0) + 4, yy + lane / 2);
      ctx.restore();
    }
  }
}

function heat(ctx: CanvasRenderingContext2D, counts: number[], y: number, h: number, g: ReturnType<typeof bucketGeometry>, width: number, base: string) {
  let max = 0;
  for (const v of counts) if (v > max) max = v;
  if (max <= 0) return;
  const w = Math.max(1, g.bucketPx);
  for (let i = 0; i < counts.length; i++) {
    const v = counts[i] ?? 0;
    if (v <= 0) continue;
    const x = g.x0 + i * g.bucketPx;
    if (x + w < 0 || x > width) continue;
    ctx.fillStyle = withAlpha(base, 0.2 + 0.7 * (v / max));
    ctx.fillRect(x, y + 4, w, h - 8);
  }
}

/** Time ruler with tick labels, the playhead handle and mark A. */
export function drawRuler(ctx: CanvasRenderingContext2D, d: RulerInput) {
  const { width, view } = d;
  ctx.clearRect(0, 0, width, RULER_H);
  const step = tickStep(view, width);
  ctx.font = "10px ui-monospace, 'Cascadia Code', Consolas, monospace";
  ctx.textBaseline = "middle";
  for (const t of ticks(view, step)) {
    const x = Math.round(xOf(t, view, width)) + 0.5;
    ctx.fillStyle = C.grid;
    ctx.fillRect(x, RULER_H - 7, 1, 7);
    ctx.fillStyle = C.muted;
    ctx.fillText(formatTick(t, step), x + 4, RULER_H / 2 - 2);
  }
  ctx.fillStyle = C.grid;
  ctx.fillRect(0, RULER_H - 1, width, 1);
  rangeBand(ctx, d.range, view, width, RULER_H, false);

  if (d.hoverX !== null) {
    ctx.fillStyle = "rgba(230, 236, 245, 0.35)";
    ctx.fillRect(Math.round(d.hoverX), 0, 1, RULER_H);
  }
  if (d.markA !== null) {
    const x = xOf(d.markA, view, width);
    if (x >= 0 && x <= width) {
      ctx.fillStyle = C.warn;
      ctx.fillRect(Math.round(x), 4, 1.5, RULER_H - 4);
      ctx.font = "bold 9px Inter, 'Segoe UI', sans-serif";
      ctx.fillText("A", Math.round(x) + 3, 8);
    }
  }
  const px = xOf(d.cursor, view, width);
  if (px >= -6 && px <= width + 6) {
    const x = Math.round(px);
    ctx.fillStyle = C.accent2;
    ctx.shadowColor = C.accent2;
    ctx.shadowBlur = 8;
    roundRect(ctx, x - 5, 2, 10, RULER_H - 9, 3);
    ctx.fill();
    ctx.beginPath();
    ctx.moveTo(x - 5, RULER_H - 8);
    ctx.lineTo(x + 5, RULER_H - 8);
    ctx.lineTo(x, RULER_H - 2);
    ctx.closePath();
    ctx.fill();
    ctx.fillRect(x - 0.75, RULER_H - 3, 1.5, 3);
    ctx.shadowBlur = 0;
  }
}

export function draw(ctx: CanvasRenderingContext2D, d: DrawInput) {
  const { width, rows, view } = d;
  const height = totalHeight(rows);
  ctx.clearRect(0, 0, width, height);

  rows.forEach((r, i) => {
    if (i % 2 === 1) {
      ctx.fillStyle = C.rowAlt;
      ctx.fillRect(0, rowTop(rows, i), width, r.height);
    }
  });

  const step = tickStep(view, width);
  ctx.fillStyle = C.grid;
  for (const t of ticks(view, step)) ctx.fillRect(Math.round(xOf(t, view, width)) + 0.5, 0, 1, height);

  const t = d.tracks;
  const g = t ? bucketGeometry(t, view, width) : null;
  rows.forEach((r, i) => {
    const y = rowTop(rows, i);
    ctx.globalAlpha = r.dimmed ? DIMMED_ROW_ALPHA : 1;
    if (r.kind === "group") {
      ctx.fillStyle = C.grid;
      ctx.fillRect(0, y + r.height - 1, width, 1);
      return;
    }
    if (r.kind === "actions") {
      ctx.font = "10px Inter, 'Segoe UI', sans-serif";
      for (const a of d.actions) {
        const x1 = xOf(a.from, view, width);
        const x2 = Math.max(x1 + 3, xOf(a.to, view, width));
        if (x2 < 0 || x1 > width) continue;
        const color = actionColor(a.state);
        roundRect(ctx, x1, y + 3, x2 - x1, r.height - 6, 3);
        ctx.fillStyle = color;
        ctx.globalAlpha = 0.85;
        ctx.fill();
        ctx.globalAlpha = 1;
        if (x2 - x1 > 60) {
          ctx.fillStyle = "#06101f";
          ctx.fillText(`${a.title} · ${a.target}`, Math.max(x1, 0) + 4, y + r.height / 2);
        }
      }
      return;
    }
    if (!t || !g) return;
    if (r.kind === "events") {
      bars(ctx, t.events, y, r.height, g, width, "rgba(34, 211, 238, 0.55)");
      for (const m of t.markers) {
        const x = g.x0 + m.time_ms * 1e6 * g.pxPerNs;
        if (x < -4 || x > width + 4) continue;
        ctx.fillStyle = C.warn;
        ctx.beginPath();
        ctx.moveTo(x, y + 3);
        ctx.lineTo(x + 4, y + r.height / 2);
        ctx.lineTo(x, y + r.height - 3);
        ctx.lineTo(x - 4, y + r.height / 2);
        ctx.closePath();
        ctx.fill();
      }
    } else if (r.kind === "errors") {
      bars(ctx, t.errors, y, r.height, g, width, C.error);
    } else if (r.kind === "requests") {
      bars(ctx, t.requests, y, r.height, g, width, C.accent);
    } else if (r.kind === "latency") {
      latency(ctx, t.latency_p95_ms, y, r.height, g);
    } else if (r.kind === "service" && r.track !== undefined) {
      const s = t.services[r.track];
      if (!s) return;
      const color = r.color ?? C.accent;
      if (s.clipped) clips(ctx, t, r.track, y, r.height, g, width, color);
      else heat(ctx, s.counts, y, r.height, g, width, color);
    }
  });
  ctx.globalAlpha = 1;
  rangeBand(ctx, d.range, view, width, height, true);

  if (d.markA !== null) {
    const x = xOf(d.markA, view, width);
    if (x >= 0 && x <= width) {
      ctx.fillStyle = C.warn;
      ctx.fillRect(Math.round(x), 0, 1.5, height);
    }
  }

  if (d.hoverX !== null) {
    ctx.fillStyle = "rgba(230, 236, 245, 0.25)";
    ctx.fillRect(Math.round(d.hoverX), 0, 1, height);
  }

  const px = xOf(d.cursor, view, width);
  if (px >= -1 && px <= width + 1) {
    const x = Math.round(px);
    ctx.fillStyle = C.accent2;
    ctx.shadowColor = C.accent2;
    ctx.shadowBlur = 8;
    ctx.fillRect(x - 0.75, 0, 1.5, height);
    ctx.shadowBlur = 0;
  }
}
