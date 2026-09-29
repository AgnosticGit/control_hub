import { describe, expect, it } from "vitest";
import { MIN_PLAY_MS, speedFor, traceRange } from "./trace-focus";
import { SPEEDS } from "./useWorkspaceEngine";

const MS = 1_000_000n;

describe("speedFor", () => {
  it("picks the fastest speed at which the request still takes a few seconds", () => {
    // 20 ms at 0.001x = 20 s; 0.01x would be 2 s (too fast to follow).
    expect(speedFor(20n * MS)).toBe(0.001);
    // 400 ms: 0.1x = 4 s.
    expect(speedFor(400n * MS)).toBe(0.1);
    expect(speedFor(5000n * MS)).toBe(1);
    expect(speedFor(3_600_000n * MS)).toBe(10);
  });

  it("falls back to the slowest speed for tiny or empty requests", () => {
    expect(speedFor(0n)).toBe(SPEEDS[0]);
    expect(speedFor(1000n)).toBe(SPEEDS[0]);
  });

  it("never plays faster than MIN_PLAY_MS unless even the slowest speed must", () => {
    for (const ms of [1, 10, 50, 399, 401, 2000, 60_000]) {
      const s = speedFor(BigInt(ms) * MS);
      if (s !== SPEEDS[0]) expect(ms / s).toBeGreaterThanOrEqual(MIN_PLAY_MS);
    }
  });
});

describe("traceRange", () => {
  const bounds = { from: 0n, to: 10_000n * MS };

  it("covers every span with 5% context on each side", () => {
    const r = traceRange(
      [
        { start_ns: String(1000n * MS), end_ns: String(1100n * MS) },
        { start_ns: String(1010n * MS), end_ns: String(1200n * MS) },
      ],
      bounds,
    );
    expect(r).toEqual({ from: 990n * MS, to: 1210n * MS });
  });

  it("keeps a minimum context for very short requests", () => {
    const r = traceRange([{ start_ns: String(1000n * MS), end_ns: String(1000n * MS + 1000n) }], bounds);
    expect(r).toEqual({ from: 1000n * MS - 50_000n, to: 1000n * MS + 51_000n });
  });

  it("clamps to the loaded range and rejects what is outside it", () => {
    expect(traceRange([{ start_ns: "0", end_ns: String(100n * MS) }], bounds)?.from).toBe(0n);
    expect(traceRange([{ start_ns: String(20_000n * MS), end_ns: String(20_001n * MS) }], bounds)).toBeNull();
  });

  it("ignores malformed timestamps and returns null without spans", () => {
    expect(traceRange([], bounds)).toBeNull();
    expect(traceRange([{ start_ns: "x", end_ns: "y" }], bounds)).toBeNull();
  });
});
