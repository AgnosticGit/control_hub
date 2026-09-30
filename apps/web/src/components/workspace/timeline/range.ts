// Pure helpers for the replay range (loop) and the timeline tooltip placement.

import type { View } from "./scale";

/** Narrowest selectable replay range. */
export const MIN_LOOP_NS = 1_000n;

/** Ordered range between two picked times, clamped to `bounds`; null when too narrow. */
export function rangeBetween(a: bigint, b: bigint, bounds: View): View | null {
  let from = a < b ? a : b;
  let to = a < b ? b : a;
  if (from < bounds.from) from = bounds.from;
  if (to > bounds.to) to = bounds.to;
  return to - from >= MIN_LOOP_NS ? { from, to } : null;
}

/**
 * Next replay cursor after advancing by `advance`. Without a loop replay stops at the end of
 * the loaded range; with a loop it wraps from the loop end back to its start.
 */
export function replayStep(cursor: bigint, advance: bigint, rangeTo: bigint, loop: View | null): { cursor: bigint; stop: boolean } {
  const next = cursor + advance;
  if (loop) {
    if (next >= loop.to) {
      const len = loop.to - loop.from;
      const over = len > 0n ? (next - loop.to) % len : 0n;
      return { cursor: loop.from + over, stop: false };
    }
    return { cursor: next < loop.from ? loop.from : next, stop: false };
  }
  return next >= rangeTo ? { cursor: rangeTo, stop: true } : { cursor: next, stop: false };
}

/** Where Play starts: the loop start when the cursor is outside it, else where it is. */
export function playStart(cursor: bigint, rangeFrom: bigint, rangeTo: bigint, loop: View | null): bigint {
  if (loop) return cursor < loop.from || cursor >= loop.to ? loop.from : cursor;
  return cursor >= rangeTo ? rangeFrom : cursor;
}

/** Where a replay that starts over begins: the first recorded item in the range, else its start. */
export function firstItemStart(first: bigint | null, rangeFrom: bigint, rangeTo: bigint): bigint {
  return first !== null && first >= rangeFrom && first < rangeTo ? first : rangeFrom;
}

/** The view that shows `range` with a little context on each side. */
export function viewAround(range: View, bounds: View): View {
  const pad = (range.to - range.from) / 10n;
  const from = range.from - pad < bounds.from ? bounds.from : range.from - pad;
  const to = range.to + pad > bounds.to ? bounds.to : range.to + pad;
  return { from, to };
}

export interface Placement {
  left: number;
  top: number;
}

/**
 * Top-left of a `w` × `h` tooltip centred directly above the pointer at (x, y), kept inside a
 * `vw` × `vh` viewport; flipped below the pointer when there is no room above.
 */
export function placeAbove(x: number, y: number, w: number, h: number, vw: number, vh: number, gap = 12, margin = 6): Placement {
  const left = Math.max(margin, Math.min(x - w / 2, vw - w - margin));
  let top = y - gap - h;
  if (top < margin) top = y + gap + 8;
  top = Math.max(margin, Math.min(top, vh - h - margin));
  return { left, top };
}
