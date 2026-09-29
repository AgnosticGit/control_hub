import { describe, expect, it } from "vitest";
import { placeAbove, playStart, rangeBetween, replayStep, viewAround } from "./range";

const bounds = { from: 1000n, to: 2_000_000n };

describe("rangeBetween", () => {
  it("orders the ends and clamps them to the bounds", () => {
    expect(rangeBetween(50_000n, 10_000n, bounds)).toEqual({ from: 10_000n, to: 50_000n });
    expect(rangeBetween(0n, 3_000_000n, bounds)).toEqual(bounds);
  });

  it("rejects a range narrower than a microsecond", () => {
    expect(rangeBetween(10_000n, 10_500n, bounds)).toBeNull();
  });
});

describe("replayStep", () => {
  it("stops at the end of the loaded range without a loop", () => {
    expect(replayStep(1_990_000n, 20_000n, bounds.to, null)).toEqual({ cursor: bounds.to, stop: true });
    expect(replayStep(10_000n, 5_000n, bounds.to, null)).toEqual({ cursor: 15_000n, stop: false });
  });

  it("wraps from the loop end to its start and keeps running", () => {
    const loop = { from: 100_000n, to: 200_000n };
    expect(replayStep(195_000n, 10_000n, bounds.to, loop)).toEqual({ cursor: 105_000n, stop: false });
    expect(replayStep(150_000n, 10_000n, bounds.to, loop)).toEqual({ cursor: 160_000n, stop: false });
  });

  it("pulls a cursor before the loop to its start", () => {
    expect(replayStep(10_000n, 1n, bounds.to, { from: 100_000n, to: 200_000n }).cursor).toBe(100_000n);
  });
});

describe("playStart", () => {
  it("starts at the loop start when the cursor is outside the loop", () => {
    const loop = { from: 100_000n, to: 200_000n };
    expect(playStart(50_000n, bounds.from, bounds.to, loop)).toBe(100_000n);
    expect(playStart(200_000n, bounds.from, bounds.to, loop)).toBe(100_000n);
    expect(playStart(150_000n, bounds.from, bounds.to, loop)).toBe(150_000n);
  });

  it("restarts from the beginning at the end of the range without a loop", () => {
    expect(playStart(bounds.to, bounds.from, bounds.to, null)).toBe(bounds.from);
  });
});

describe("viewAround", () => {
  it("adds 10% context on each side within the bounds", () => {
    expect(viewAround({ from: 100_000n, to: 200_000n }, bounds)).toEqual({ from: 90_000n, to: 210_000n });
    expect(viewAround({ from: 1000n, to: 101_000n }, bounds).from).toBe(1000n);
  });
});

describe("placeAbove", () => {
  it("centres the tooltip right above the pointer", () => {
    expect(placeAbove(500, 400, 200, 60, 1200, 800)).toEqual({ left: 400, top: 328 });
  });

  it("keeps the tooltip inside the left and right edges", () => {
    expect(placeAbove(10, 400, 200, 60, 1200, 800).left).toBe(6);
    expect(placeAbove(1195, 400, 200, 60, 1200, 800).left).toBe(994);
  });

  it("flips below the pointer when there is no room above", () => {
    const p = placeAbove(500, 30, 200, 60, 1200, 800);
    expect(p.top).toBeGreaterThan(30);
  });
});
