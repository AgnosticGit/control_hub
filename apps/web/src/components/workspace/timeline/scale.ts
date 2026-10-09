// Pure math for the zoomable timeline: view window, zoom/pan and ruler ticks.
// Times are BigInt nanoseconds; JS numbers are used only for pixel math on
// differences inside the window.

export interface View {
  from: bigint;
  to: bigint;
}

const US = 1_000n;
const MS = 1_000_000n;
const SEC = 1_000_000_000n;
const MIN = 60n * SEC;
const HOUR = 60n * MIN;
const DAY = 24n * HOUR;

/** Narrowest view: one millisecond. */
export const MIN_VIEW_NS = MS;
/** Widest view: one month. Navigation can continue through adjacent ranges. */
export const MAX_VIEW_NS = 30n * DAY;
/** Raw rows stay bounded to the API's seven-day query window. */
export const MAX_DETAIL_RANGE_NS = 7n * DAY;

/** Pick the raw-data window needed for a close zoom into a broader view. */
export function detailChunkFor(view: View): View {
  const width = span(view);
  if (width > MAX_DETAIL_RANGE_NS) {
    const middle = view.from + width / 2n;
    const from = middle - MAX_DETAIL_RANGE_NS / 2n;
    return { from, to: from + MAX_DETAIL_RANGE_NS };
  }
  // Align detail windows so small pans reuse a previous seven-day fetch.
  let from = (view.from / MAX_DETAIL_RANGE_NS) * MAX_DETAIL_RANGE_NS;
  if (view.to > from + MAX_DETAIL_RANGE_NS) from = view.to - MAX_DETAIL_RANGE_NS;
  return { from, to: from + MAX_DETAIL_RANGE_NS };
}

/** True when one or more loaded intervals continuously cover the requested view. */
export function rangeCoveredBy(loaded: readonly View[], target: View): boolean {
  let coveredTo = target.from;
  for (const range of [...loaded].sort((a, b) => (a.from < b.from ? -1 : a.from > b.from ? 1 : 0))) {
    if (range.to <= coveredTo) continue;
    if (range.from > coveredTo) return false;
    coveredTo = range.to;
    if (coveredTo >= target.to) return true;
  }
  return false;
}

const TICK_STEPS: bigint[] = [
  1n * US, 2n * US, 5n * US, 10n * US, 20n * US, 50n * US, 100n * US, 200n * US, 500n * US,
  1n * MS, 2n * MS, 5n * MS, 10n * MS, 20n * MS, 50n * MS, 100n * MS, 200n * MS, 500n * MS,
  1n * SEC, 2n * SEC, 5n * SEC, 10n * SEC, 15n * SEC, 30n * SEC,
  1n * MIN, 2n * MIN, 5n * MIN, 10n * MIN, 15n * MIN, 30n * MIN,
  1n * HOUR, 2n * HOUR, 3n * HOUR, 6n * HOUR, 12n * HOUR, 24n * HOUR,
  2n * DAY, 3n * DAY, 7n * DAY, 14n * DAY, 30n * DAY,
];

const span = (v: View) => v.to - v.from;

/** Keeps `v` inside `bounds` without changing its width (unless wider than the bounds). */
export function clampView(v: View, bounds: View): View {
  const w = span(v);
  const bw = span(bounds);
  if (w >= bw) return { ...bounds };
  if (v.from < bounds.from) return { from: bounds.from, to: bounds.from + w };
  if (v.to > bounds.to) return { from: bounds.to - w, to: bounds.to };
  return v;
}

function zoomedWidth(v: View, factor: number, bounds: View): bigint {
  const bw = span(bounds);
  const minW = bw < MIN_VIEW_NS ? bw : MIN_VIEW_NS;
  const maxW = bw < MAX_VIEW_NS ? bw : MAX_VIEW_NS;
  const next = BigInt(Math.round(Number(span(v)) * factor));
  return next < minW ? minW : next > maxW ? maxW : next;
}

/** Zooms by `factor` (< 1 zooms in) keeping `anchor` at the same screen position. */
export function zoomAt(v: View, anchor: bigint, factor: number, bounds: View): View {
  const w = span(v);
  const next = zoomedWidth(v, factor, bounds);
  const a = anchor < v.from ? v.from : anchor > v.to ? v.to : anchor;
  const frac = w > 0n ? Number(a - v.from) / Number(w) : 0.5;
  const from = a - BigInt(Math.round(Number(next) * frac));
  return clampView({ from, to: from + next }, bounds);
}

/** Zooms by `factor` and centres the window on `center` (as far as the bounds allow). */
export function zoomCentered(v: View, center: bigint, factor: number, bounds: View): View {
  const next = zoomedWidth(v, factor, bounds);
  const from = center - next / 2n;
  return clampView({ from, to: from + next }, bounds);
}

export function panBy(v: View, deltaNs: bigint, bounds: View): View {
  return clampView({ from: v.from + deltaNs, to: v.to + deltaNs }, bounds);
}

/** Everything but a fixed historical range can continue advancing with live data. */
export function hasFuture(mode: string): boolean {
  return mode !== "historical";
}

/**
 * Live: keeps the playhead at the right edge when it leaves the current view.
 */
export function followLive(v: View, t: bigint, bounds: View): View {
  const now = inside(t, bounds);
  if (now >= v.from && now <= v.to) return v;
  const from = now - span(v);
  return clampView({ from, to: from + span(v) }, bounds);
}

/**
 * A time outside `bounds` is followed as far as the bounds go: the view can never contain it,
 * so chasing it would move the view on every call.
 */
function inside(t: bigint, bounds: View): bigint {
  return t < bounds.from ? bounds.from : t > bounds.to ? bounds.to : t;
}

/** When `t` leaves the view, page so that it sits a quarter into the window. */
export function follow(v: View, time: bigint, bounds: View): View {
  const t = inside(time, bounds);
  if (t >= v.from && t <= v.to) return v;
  const w = span(v);
  const from = t - w / 4n;
  return clampView({ from, to: from + w }, bounds);
}

export function xOf(t: bigint, v: View, width: number): number {
  const w = span(v);
  return w > 0n ? (Number(t - v.from) / Number(w)) * width : 0;
}

/** The playhead's interactive hit target, shared by the ruler and every track row. */
export function isNearPlayhead(x: number, cursor: bigint, view: View, width: number, tolerance = 8): boolean {
  return width > 0 && Math.abs(x - xOf(cursor, view, width)) <= tolerance;
}

export function timeAt(x: number, v: View, width: number): bigint {
  if (width <= 0) return v.from;
  return v.from + BigInt(Math.round((Number(span(v)) * x) / width));
}

/** Smallest step whose ticks are at least `minPx` apart. */
export function tickStep(v: View, width: number, minPx = 90): bigint {
  const w = Number(span(v));
  for (const s of TICK_STEPS) {
    if (width > 0 && (Number(s) / w) * width >= minPx) return s;
  }
  return TICK_STEPS[TICK_STEPS.length - 1] ?? HOUR;
}

/** Tick times inside the view, aligned to `step` in local time. */
export function ticks(v: View, step: bigint): bigint[] {
  const offset = BigInt(new Date(Number(v.from / MS)).getTimezoneOffset()) * MIN;
  const local = v.from - offset;
  let t = local - (((local % step) + step) % step) + offset;
  if (t < v.from) t += step;
  const out: bigint[] = [];
  for (; t <= v.to && out.length < 200; t += step) out.push(t);
  return out;
}

const pad = (n: number, w = 2) => String(n).padStart(w, "0");

/** Label precision follows the step: `HH:MM`, `HH:MM:SS`, `MM:SS.mmm` or `SS.mmm uuu`. */
export function formatTick(t: bigint, step: bigint): string {
  const d = new Date(Number(t / MS));
  if (step >= DAY) return new Intl.DateTimeFormat("en", { month: "short", day: "numeric" }).format(d);
  const hm = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (step >= MIN) return hm;
  if (step >= SEC) return `${hm}:${pad(d.getSeconds())}`;
  const ms = pad(d.getMilliseconds(), 3);
  if (step >= MS) return `${pad(d.getMinutes())}:${pad(d.getSeconds())}.${ms}`;
  const us = pad(Number((((t % MS) + MS) % MS) / US), 3);
  return `${pad(d.getSeconds())}.${ms} ${us}`;
}

/** Human width of the view, e.g. `2.5 ms`, `40 s`, `1.5 h`. */
export function formatSpan(ns: bigint): string {
  const n = Number(ns);
  if (n < 1e6) return `${(n / 1e3).toFixed(0)} µs`;
  if (n < 1e9) return `${+(n / 1e6).toFixed(n < 1e7 ? 2 : 0)} ms`;
  if (n < 60e9) return `${+(n / 1e9).toFixed(n < 1e10 ? 2 : 1)} s`;
  if (n < 3600e9) return `${+(n / 60e9).toFixed(1)} min`;
  if (n >= 24 * 3600e9) return `${+(n / (24 * 3600e9)).toFixed(n < 2 * 24 * 3600e9 ? 1 : 0)} days`;
  return `${+(n / 3600e9).toFixed(1)} h`;
}
