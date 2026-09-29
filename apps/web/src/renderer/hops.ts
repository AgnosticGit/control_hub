// Where a recorded request hop is at a given moment. Pure functions, no Pixi, so they are
// unit-tested. Times are milliseconds relative to the frame time (see `HOP_STRIDE`).

/**
 * Where a call is at time `now`, from its four timestamps (request leaves the caller at t0,
 * reaches the callee at t1, the answer leaves at t2 and is back at t3):
 * - `request`: the dot goes caller (p 0) -> callee (p 1) over t0..t1;
 * - `wait`: the callee is working (t1..t2); the request is parked at the callee (p 1);
 * - `response`: the answer goes callee (p 1) -> caller (p 0) over t2..t3.
 * A leaf call (target not instrumented) has no t1/t2 of its own; its two legs split the call
 * evenly, which is an assumption, not a measurement. Null outside t0..t3.
 */
export type HopPhase = { dir: "request" | "wait" | "response"; p: number };

export function hopPhase(t0: number, t1: number, t2: number, t3: number, now: number, leaf: boolean): HopPhase | null {
  if (now < t0 || now > t3) return null;
  let arrive = Math.min(Math.max(t1, t0), t3);
  let answer = Math.min(Math.max(t2, arrive), t3);
  if (leaf) {
    arrive = (t0 + t3) / 2;
    answer = arrive;
  }
  if (now < arrive) return { dir: "request", p: (now - t0) / (arrive - t0) };
  if (now < answer) return { dir: "wait", p: 1 };
  return { dir: "response", p: t3 > answer ? 1 - (now - answer) / (t3 - answer) : 1 };
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
