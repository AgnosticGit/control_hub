// When a zoomed-in view shows nothing at all, the timeline offers to jump to the nearest item
// on either side. Pure, unit-tested.

import type { TimelineTracks } from "@controlhub/web-engine";
import type { ActionBar } from "./draw";
import { clampView, type View } from "./scale";

export interface Neighbours {
  prev: bigint | null;
  next: bigint | null;
}

const sum = (xs: readonly number[]) => xs.reduce((s, x) => s + x, 0);

function parse(ns: string | null | undefined): bigint | null {
  if (!ns) return null;
  try {
    return BigInt(ns);
  } catch {
    return null;
  }
}

/** True when `tracks` were computed for exactly this view. */
export function tracksMatch(tracks: TimelineTracks | null, view: View): tracks is TimelineTracks {
  return tracks !== null && tracks.from_ns === view.from.toString() && tracks.to_ns === view.to.toString();
}

/** Nothing is drawn in the view: no request, error, event or span, and no action overlapping it. */
export function viewIsEmpty(tracks: TimelineTracks, view: View, bars: readonly ActionBar[]): boolean {
  if (sum(tracks.requests) + sum(tracks.errors) + sum(tracks.events) > 0) return false;
  if (tracks.services.some((s) => s.spans_in_view > 0)) return false;
  return !bars.some((b) => b.from <= view.to && b.to >= view.from);
}

/** Nearest item before and after the view, from the engine's tracks and the action bars. */
export function nearestItems(tracks: TimelineTracks, view: View, bars: readonly ActionBar[]): Neighbours {
  let prev = parse(tracks.prev_item_ns);
  let next = parse(tracks.next_item_ns);
  for (const b of bars) {
    if (b.to < view.from && (prev === null || b.to > prev)) prev = b.to;
    if (b.from > view.to && (next === null || b.from < next)) next = b.from;
  }
  return { prev, next };
}

/** Same width as `view`, centred on `t` as far as `bounds` allow. */
export function centeredOn(view: View, t: bigint, bounds: View): View {
  const half = (view.to - view.from) / 2n;
  return clampView({ from: t - half, to: t - half + (view.to - view.from) }, bounds);
}
