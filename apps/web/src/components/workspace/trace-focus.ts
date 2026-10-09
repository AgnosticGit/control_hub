// Pure helpers for focusing one request (trace): its replay range and a speed at which it can
// be followed by eye.

import type { View } from "./timeline/scale";
import { MIN_LOOP_NS } from "./timeline/range";
import { SPEEDS, SPEED_MIN, type Speed } from "./useWorkspaceEngine";

/** Shortest wall-clock time one pass over a focused request should take. */
export const MIN_PLAY_MS = 4000;
const NS_PER_MS = 1_000_000;
/** Context shown before and after the request, at least this much. */
const MIN_PAD_NS = 50_000n;

/** The fastest replay speed at which the request takes at least `MIN_PLAY_MS` to play. */
export function speedFor(durationNs: bigint): Speed {
  const ms = Number(durationNs) / NS_PER_MS;
  let best: Speed = SPEEDS[0] ?? SPEED_MIN;
  for (const s of SPEEDS) if (ms / s >= MIN_PLAY_MS) best = s;
  return best;
}

/**
 * Replay range covering every span of the request plus 5 % context on each side, clamped to
 * the loaded range. Null when there are no spans or the request lies outside `bounds`.
 */
export function traceRange(spans: ReadonlyArray<{ start_ns: string; end_ns: string }>, bounds: View): View | null {
  let min: bigint | null = null;
  let max: bigint | null = null;
  for (const s of spans) {
    let start: bigint;
    let end: bigint;
    try {
      start = BigInt(s.start_ns);
      end = BigInt(s.end_ns);
    } catch {
      continue;
    }
    if (min === null || start < min) min = start;
    if (max === null || end > max) max = end;
  }
  if (min === null || max === null) return null;
  let pad = (max - min) / 20n;
  if (pad < MIN_PAD_NS) pad = MIN_PAD_NS;
  const from = min - pad < bounds.from ? bounds.from : min - pad;
  const to = max + pad > bounds.to ? bounds.to : max + pad;
  return to - from >= MIN_LOOP_NS ? { from, to } : null;
}
