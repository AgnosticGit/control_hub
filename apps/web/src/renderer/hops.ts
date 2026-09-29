// Where a recorded request hop is at a given moment. Pure functions, no Pixi, so they are
// unit-tested. Times are milliseconds relative to the frame time (see `HOP_STRIDE`).

/**
 * Progress of a call at time `now`: the dot goes from the caller (0) to the callee (1) over the
 * call's recorded duration t0..t3, so the middle of the call is the middle of the line. It is
 * a progress indicator, not a network position. Null when the call is not in progress.
 */
export function callProgress(t0: number, t3: number, now: number): number | null {
  if (now < t0 || now > t3) return null;
  return t3 > t0 ? (now - t0) / (t3 - t0) : 1;
}

/**
 * How the animation time moves between frames.
 * - `wall`: live; wall-clock time minus a fixed delivery delay (telemetry arrives in batches).
 * - `frame`: replay/paused; the frame time advanced at `rate` x real time (0 = frozen).
 */
export type AnimClock = { kind: "wall"; delayMs: number } | { kind: "frame"; rate: number };

/** Animation time relative to the frame time, in ms. `frameTimeUs`: frame time in µs since epoch. */
export function animTimeMs(
  clock: AnimClock,
  frameTimeUs: number,
  frameArrivedAt: number,
  perfNow: number,
  timeOrigin: number,
): number {
  if (clock.kind === "wall") return timeOrigin + perfNow - clock.delayMs - frameTimeUs / 1000;
  return (perfNow - frameArrivedAt) * clock.rate;
}
