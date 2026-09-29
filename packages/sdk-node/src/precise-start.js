// Sub-millisecond span start times.
//
// OpenTelemetry JS stamps a span's start with `Date.now()` (1 ms resolution) and derives the
// end from a high-resolution duration. Every span of a fast request therefore "starts" in the
// same millisecond, and the order of calls inside it is lost. This processor replaces the start
// with `performance.timeOrigin` + the monotonic reading taken when the span was created. Node
// takes `timeOrigin` from the precise system clock at process start, so services on one host
// agree to microseconds; `Date.now()` can lag by up to ~1 ms on Windows and is only used to
// follow real drift or a stepped clock (more than `MAX_TRUSTED_OFFSET_NS`).
// The end and event times follow automatically: the SDK computes them as start + duration.

const NS_PER_MS = 1_000_000n;
const NS_PER_SEC = 1_000_000_000n;

/** @param {[number, number]} hr */
export function hrToNs(hr) {
  return BigInt(hr[0]) * NS_PER_SEC + BigInt(hr[1]);
}

/** @param {bigint} ns @returns {[number, number]} */
export function nsToHr(ns) {
  return [Number(ns / NS_PER_SEC), Number(ns % NS_PER_SEC)];
}

/** Epoch nanoseconds for a `performance.now()` reading. */
export function perfToEpochNs(perfMs, timeOrigin = performance.timeOrigin) {
  return BigInt(Math.round(timeOrigin * 1000)) * 1000n + BigInt(Math.round(perfMs * 1e6));
}

/** Below this estimated `Date.now()` − monotonic offset, the monotonic clock is trusted as is. */
export const MAX_TRUSTED_OFFSET_NS = 2_000_000;

/**
 * The refined start from the monotonic epoch time `mono` and the estimated offset of the wall
 * clock from it. A recorded time that is not on a millisecond boundary was given precisely
 * and is kept.
 * @param {bigint} recorded
 * @param {bigint} mono
 * @param {number} offsetNs
 */
export function refineStart(recorded, mono, offsetNs) {
  if (recorded % NS_PER_MS !== 0n) return recorded;
  return Math.abs(offsetNs) <= MAX_TRUSTED_OFFSET_NS ? mono : mono + BigInt(Math.round(offsetNs));
}

/** Allowed drift between the monotonic and the wall clock: 50 ppm, in ns per ms. */
const DRIFT_NS_PER_MS = 50;

/**
 * Estimates `wall - monotonic` in nanoseconds. Each observation of a millisecond-floored wall
 * time and a precise monotonic time bounds the offset to a 1 ms interval; intersecting them
 * narrows it well below 1 ms. Bounds widen slowly over time so clock drift is followed, and a
 * contradiction (e.g. the wall clock was stepped) restarts the estimate.
 */
export class ClockOffset {
  lo = -Infinity;
  hi = Infinity;
  at = 0;

  /** @param {bigint} wallFloorNs @param {bigint} monoEpochNs @param {number} nowMs */
  observe(wallFloorNs, monoEpochNs, nowMs) {
    const l = Number(wallFloorNs - monoEpochNs);
    const h = l + 1e6;
    if (Number.isFinite(this.lo)) {
      const widen = Math.max(0, nowMs - this.at) * DRIFT_NS_PER_MS;
      this.lo -= widen;
      this.hi += widen;
    }
    this.at = nowMs;
    let lo = Math.max(this.lo, l);
    let hi = Math.min(this.hi, h);
    if (lo > hi) {
      lo = l;
      hi = h;
    }
    this.lo = lo;
    this.hi = hi;
    return (lo + hi) / 2;
  }
}

/** Span processor that must run before the span ends; order among processors does not matter. */
export class PreciseStartSpanProcessor {
  offset = new ClockOffset();

  onStart(span) {
    // An explicit start time from the caller is kept as given.
    if (span._startTimeProvided === true) return;
    const desc = Object.getOwnPropertyDescriptor(span, 'startTime');
    if (!desc || !desc.writable || !Array.isArray(span.startTime)) return;
    const perfStart = typeof span._performanceStartTime === 'number' ? span._performanceStartTime : performance.now();
    const recorded = hrToNs(span.startTime);
    if (recorded % NS_PER_MS !== 0n) return;
    const mono = perfToEpochNs(perfStart);
    const offset = this.offset.observe(recorded, mono, perfStart);
    const refined = refineStart(recorded, mono, offset);
    if (refined !== recorded) span.startTime = nsToHr(refined);
  }

  onEnd() {}

  forceFlush() {
    return Promise.resolve();
  }

  shutdown() {
    return Promise.resolve();
  }
}
