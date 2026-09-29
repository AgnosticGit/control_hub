import { describe, expect, it } from "vitest";
import { animTimeMs, hopPhase } from "./hops";

describe("hopPhase", () => {
  // Request 0..2 ms, callee works 2..8 ms, answer 8..10 ms.
  const at = (now: number, leaf = false) => hopPhase(0, 2, 8, 10, now, leaf);

  it("is null outside the call", () => {
    expect(at(-0.001)).toBeNull();
    expect(at(10.001)).toBeNull();
  });

  it("moves the request dot from the caller to the callee over t0..t1", () => {
    expect(at(0)).toEqual({ dir: "request", p: 0 });
    expect(at(1)).toEqual({ dir: "request", p: 0.5 });
    expect(at(1.5)).toEqual({ dir: "request", p: 0.75 });
  });

  it("parks the request at the callee while it works, then sends the answer back", () => {
    expect(at(2)).toEqual({ dir: "wait", p: 1 });
    expect(at(5)).toEqual({ dir: "wait", p: 1 });
    expect(at(8)).toEqual({ dir: "response", p: 1 });
    expect(at(9)).toEqual({ dir: "response", p: 0.5 });
    // Answer sent but not received yet: still on the line, not at the caller.
    expect(at(9.9)?.p).toBeGreaterThan(0);
    expect(at(10)).toEqual({ dir: "response", p: 0 });
  });

  it("splits an uninstrumented (leaf) call evenly between the two legs", () => {
    // The engine reports a leaf as t1 = t0 and t2 = t3.
    expect(hopPhase(0, 0, 10, 10, 2.5, true)).toEqual({ dir: "request", p: 0.5 });
    expect(hopPhase(0, 0, 10, 10, 7.5, true)).toEqual({ dir: "response", p: 0.5 });
  });

  it("never runs backwards on out-of-order timestamps", () => {
    // t1 after t3 and t2 before t1 are clamped into t0..t3.
    expect(hopPhase(0, 12, 1, 10, 5, false)?.dir).toBe("request");
    expect(hopPhase(0, 12, 1, 10, 10, false)).toEqual({ dir: "response", p: 1 });
  });

  it("shows a zero-length call at the callee for that instant only", () => {
    expect(hopPhase(3, 3, 3, 3, 3, false)).toEqual({ dir: "response", p: 1 });
    expect(hopPhase(3, 3, 3, 3, 3.001, false)).toBeNull();
  });

  it("resolves microseconds", () => {
    expect(hopPhase(0, 0.004, 0.006, 0.01, 0.002, false)?.p).toBeCloseTo(0.5, 9);
    expect(hopPhase(0, 0.004, 0.006, 0.01, 0.008, false)?.p).toBeCloseTo(0.5, 9);
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
