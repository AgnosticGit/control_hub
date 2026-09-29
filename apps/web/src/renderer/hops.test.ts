import { describe, expect, it } from "vitest";
import { animTimeMs, callProgress } from "./hops";

describe("callProgress", () => {
  it("is null outside the call", () => {
    expect(callProgress(0, 10, -0.001)).toBeNull();
    expect(callProgress(0, 10, 10.001)).toBeNull();
  });

  it("puts the dot at the same fraction of the line as of the call", () => {
    expect(callProgress(0, 10, 0)).toBe(0);
    expect(callProgress(0, 10, 5)).toBe(0.5);
    expect(callProgress(0, 10, 7.5)).toBe(0.75);
    expect(callProgress(0, 10, 10)).toBe(1);
    // Database call 2..7 ms: the middle of the call is the middle of the line.
    expect(callProgress(2, 7, 4.5)).toBe(0.5);
  });

  it("resolves microseconds", () => {
    expect(callProgress(0, 0.004, 0.002)).toBeCloseTo(0.5, 9);
  });

  it("shows a zero-length call at the callee for that instant only", () => {
    expect(callProgress(3, 3, 3)).toBe(1);
    expect(callProgress(3, 3, 3.001)).toBeNull();
  });
});

describe("animTimeMs", () => {
  it("replays at the chosen rate from the frame time", () => {
    const c = { kind: "frame", rate: 0.001 } as const;
    // One real second at 0.001x advances one millisecond of recorded time.
    expect(animTimeMs(c, 0, 1000, 2000, 0)).toBeCloseTo(1, 9);
    expect(animTimeMs({ kind: "frame", rate: 0 }, 0, 1000, 5000, 0)).toBe(0);
  });

  it("follows the wall clock minus the delivery delay in live mode", () => {
    const frameTimeUs = 1_790_000_000_000_000; // µs since epoch
    const timeOrigin = 1_790_000_000_000 - 100; // ms
    // perfNow 100 ms -> wall time equals the frame time; minus 8 s delay.
    expect(animTimeMs({ kind: "wall", delayMs: 8000 }, frameTimeUs, 0, 100, timeOrigin)).toBeCloseTo(-8000, 3);
  });
});
