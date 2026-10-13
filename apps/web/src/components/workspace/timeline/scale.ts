// Pure math for the zoomable timeline: view window, zoom/pan and ruler ticks.
// Times are BigInt nanoseconds; JS numbers are used only for pixel math on
// differences inside the window.

export interface View {
  from: bigint;
  to: bigint;
}

export type TimelineStartBoundary = { status: "unknown" } | { status: "known"; oldestNs: bigint | null };

/** Clamp leftward navigation only after the server confirms the history boundary. */
export function timelineBounds(boundary: TimelineStartBoundary, latest: bigint): View {
  const from = boundary.status === "unknown" ? 0n : boundary.oldestNs ?? latest - MAX_VIEW_NS;
  return { from: from > latest ? latest : from, to: latest };
}

export interface VisibleRangeSegment {
  left: number;
  width: number;
}

/** Expand one contiguous loaded/requested window while preserving all prior coverage. */
export function expandRange(previous: View | null, requested: View): View {
  if (!previous) return { ...requested };
  return {
    from: previous.from < requested.from ? previous.from : requested.from,
    to: previous.to > requested.to ? previous.to : requested.to,
  };
}

/** Maps the part of a loading range visible in the current timeline view to pixels. */
export function visibleRangeSegment(range: View, view: View, width: number): VisibleRangeSegment | null {
  if (width <= 0 || range.to <= view.from || range.from >= view.to) return null;
  const from = range.from < view.from ? view.from : range.from;
  const to = range.to > view.to ? view.to : range.to;
  const left = xOf(from, view, width);
  return { left, width: xOf(to, view, width) - left };
}

/** Merge loaded time intervals so adjacent live frames form one covered timeline. */
export function mergeRanges(ranges: readonly View[], addition: View): View[] {
  const sorted = [...ranges, addition].sort((a, b) => (a.from < b.from ? -1 : a.from > b.from ? 1 : 0));
  const merged: View[] = [];
  for (const range of sorted) {
    const previous = merged[merged.length - 1];
    if (!previous || range.from > previous.to) merged.push({ ...range });
    else if (range.to > previous.to) merged[merged.length - 1] = { from: previous.from, to: range.to };
  }
  return merged;
}

/** Drop evicted coverage and clip a surviving interval at the engine's retention edge. */
export function trimRangesBefore(ranges: readonly View[], cutoff: bigint): View[] {
  return ranges.flatMap((range) => range.to <= cutoff ? [] : [{ from: range.from < cutoff ? cutoff : range.from, to: range.to }]);
}

/** In non-live modes, keep every interval the user has already loaded in the engine. */
export function timelineEvictionFloor(
  mode: "live" | "paused" | "replay" | "historical",
  rangeFrom: bigint,
  rollingFrom: bigint,
  loaded: readonly View[],
): bigint {
  if (mode === "live") return rollingFrom;
  let floor = rangeFrom < rollingFrom ? rangeFrom : rollingFrom;
  for (const range of loaded) if (range.from < floor) floor = range.from;
  return floor;
}

/** Keep a point request indicator local to the selected time, even when its fetch is a large chunk. */
export function pointLoadingRange(time: bigint, bounds: View, radius = 2n * SEC): View {
  const from = time - radius < bounds.from ? bounds.from : time - radius;
  const to = time + radius > bounds.to ? bounds.to : time + radius;
  return to > from ? { from, to } : { from: bounds.from, to: bounds.to };
}

const US = 1_000n;
const MS = 1_000_000n;
const SEC = 1_000_000_000n;
const LIVE_SUMMARY_STEP_NS = 10n * SEC;
const MIN = 60n * SEC;
const HOUR = 60n * MIN;
const DAY = 24n * HOUR;

/** Stable, 10-second-aligned summary bounds prevent a new live query on every cursor tick. */
export function liveSummaryWindow(view: View): View {
  const floorToStep = (value: bigint) => {
    const remainder = value % LIVE_SUMMARY_STEP_NS;
    return remainder < 0n ? value - remainder - LIVE_SUMMARY_STEP_NS : value - remainder;
  };
  const from = floorToStep(view.from);
  const toFloor = floorToStep(view.to);
  const to = toFloor === view.to ? view.to : toFloor + LIVE_SUMMARY_STEP_NS;
  return { from, to };
}

/** Opening live view: one hour of summarized history, while the engine keeps its smaller raw window. */
export const DEFAULT_LIVE_VIEW_NS = HOUR;
export const LIVE_DETAIL_WINDOW_NS = 15n * MIN;

export function defaultLiveView(now: bigint): View {
  return { from: now - DEFAULT_LIVE_VIEW_NS, to: now };
}

/** Reject future-only windows and clamp requests that cross the present. */
export function clampRangeToNow(view: View, now: bigint): View | null {
  if (view.from >= now) return null;
  return { from: view.from, to: view.to > now ? now : view.to };
}

/** Narrowest view: one millisecond. */
export const MIN_VIEW_NS = MS;
/** Widest view: one month. Navigation can continue through adjacent ranges. */
export const MAX_VIEW_NS = 30n * DAY;
/** Raw rows stay bounded to the API's seven-day query window. */
export const MAX_DETAIL_RANGE_NS = 7n * DAY;

/** Pick the raw-data window needed for a close zoom into a broader view. */
export function detailChunkFor(view: View, maxTo?: bigint): View {
  const width = span(view);
  if (width > MAX_DETAIL_RANGE_NS) {
    const middle = view.from + width / 2n;
    const from = middle - MAX_DETAIL_RANGE_NS / 2n;
    return capChunk({ from, to: from + MAX_DETAIL_RANGE_NS }, maxTo);
  }
  // Align detail windows so small pans reuse a previous seven-day fetch.
  let from = (view.from / MAX_DETAIL_RANGE_NS) * MAX_DETAIL_RANGE_NS;
  if (view.to > from + MAX_DETAIL_RANGE_NS) from = view.to - MAX_DETAIL_RANGE_NS;
  return capChunk({ from, to: from + MAX_DETAIL_RANGE_NS }, maxTo);
}

function capChunk(chunk: View, maxTo?: bigint): View {
  if (maxTo === undefined || chunk.to <= maxTo) return chunk;
  const to = maxTo;
  return { from: to - MAX_DETAIL_RANGE_NS, to };
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

/** Returns only the gaps that still need loading inside a requested interval. */
export function uncoveredRanges(loaded: readonly View[], target: View): View[] {
  const ranges = [...loaded].sort((a, b) => (a.from < b.from ? -1 : a.from > b.from ? 1 : 0));
  const gaps: View[] = [];
  let cursor = target.from;
  for (const range of ranges) {
    if (range.to <= cursor) continue;
    if (range.from >= target.to) break;
    if (range.from > cursor) gaps.push({ from: cursor, to: range.from < target.to ? range.from : target.to });
    if (range.to > cursor) cursor = range.to;
    if (cursor >= target.to) return gaps;
  }
  if (cursor < target.to) gaps.push({ from: cursor, to: target.to });
  return gaps;
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

/** Aggregated timeline data keeps wider windows complete without enlarging the engine's raw buffer. */
export function needsTimelineSummary(view: View): boolean {
  return span(view) > LIVE_DETAIL_WINDOW_NS;
}

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
