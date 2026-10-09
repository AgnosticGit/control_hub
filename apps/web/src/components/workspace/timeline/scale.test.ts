import { describe, expect, it } from "vitest";
import {
  MIN_VIEW_NS,
  follow,
  followLive,
  formatSpan,
  formatTick,
  panBy,
  tickStep,
  ticks,
  timeAt,
  xOf,
  zoomAt,
  zoomCentered,
} from "./scale";

const MS = 1_000_000n;
const SEC = 1_000_000_000n;
const T0 = 1_790_000_000n * SEC;
const bounds = { from: T0, to: T0 + 900n * SEC };

describe("followLive", () => {
  const v = { from: T0 + 100n * SEC, to: T0 + 200n * SEC };

  it("leaves the view alone while the playhead is inside it", () => {
    expect(followLive(v, T0 + 150n * SEC, bounds)).toBe(v);
    expect(followLive(v, T0 + 190n * SEC, bounds)).toBe(v);
  });

  it("moves the window so the live playhead sits on the right edge", () => {
    const moved = followLive(v, T0 + 201n * SEC, bounds);
    expect(moved.to - moved.from).toBe(100n * SEC);
    expect(xOf(T0 + 201n * SEC, moved, 1000)).toBeCloseTo(1000, 6);
    expect(followLive(moved, T0 + 201n * SEC, bounds)).toBe(moved);
  });

  it("jumps back to now when the view was scrolled into the past", () => {
    const back = followLive(v, T0 + 500n * SEC, bounds);
    expect(xOf(T0 + 500n * SEC, back, 1000)).toBeCloseTo(1000, 6);
  });

  it("stays inside the shown range", () => {
    const end = followLive(v, bounds.to, bounds);
    expect(end.to).toBe(bounds.to);
  });
});

describe("zoomCentered", () => {
  it("centres the window on the cursor, within the loaded range", () => {
    const z = zoomCentered(bounds, T0 + 300n * SEC, 0.1, bounds);
    expect(z.to - z.from).toBe((bounds.to - bounds.from) / 10n);
    expect(z.from + (z.to - z.from) / 2n).toBe(T0 + 300n * SEC);
    const edge = zoomCentered(bounds, bounds.to, 0.1, bounds);
    expect(edge.to).toBe(bounds.to);
  });
});

describe("zoomAt", () => {
  it("keeps the anchor at the same screen position", () => {
    const v = { from: T0 + 100n * SEC, to: T0 + 200n * SEC };
    const anchor = T0 + 125n * SEC;
    const z = zoomAt(v, anchor, 0.5, bounds);
    expect(z.to - z.from).toBe(50n * SEC);
    expect(xOf(anchor, z, 1000)).toBeCloseTo(xOf(anchor, v, 1000), 6);
  });

  it("stops at one millisecond and at the loaded range", () => {
    let v = { ...bounds };
    for (let i = 0; i < 100; i++) v = zoomAt(v, T0 + 450n * SEC, 0.25, bounds);
    expect(v.to - v.from).toBe(MIN_VIEW_NS);
    for (let i = 0; i < 100; i++) v = zoomAt(v, T0, 4, bounds);
    expect(v).toEqual(bounds);
  });
});

describe("panBy and follow", () => {
  it("clamps to the loaded range without changing width", () => {
    const v = { from: T0 + 10n * SEC, to: T0 + 20n * SEC };
    expect(panBy(v, -60n * SEC, bounds)).toEqual({ from: T0, to: T0 + 10n * SEC });
    expect(panBy(v, 10_000n * SEC, bounds)).toEqual({ from: bounds.to - 10n * SEC, to: bounds.to });
  });

  it("pages so the playhead sits a quarter into the window", () => {
    const v = { from: T0, to: T0 + 8n * MS };
    expect(follow(v, T0 + 4n * MS, bounds)).toBe(v);
    const f = follow(v, T0 + 20n * MS, bounds);
    expect(f).toEqual({ from: T0 + 18n * MS, to: T0 + 26n * MS });
  });

  it("settles when the playhead is outside the loaded range", () => {
    const v = { from: T0 + 300n * SEC, to: T0 + 600n * SEC };
    for (const t of [T0 - 3n * MS, bounds.to + 5n * SEC]) {
      const live = (view: typeof v, time: bigint, b: typeof bounds) => followLive(view, time, b);
      for (const move of [follow, live]) {
        const once = move(v, t, bounds);
        expect(move(once, t, bounds)).toBe(once);
        expect(once.from >= bounds.from && once.to <= bounds.to).toBe(true);
      }
    }
  });
});

describe("ticks", () => {
  it("chooses finer steps as the view narrows, down to microseconds", () => {
    expect(tickStep({ from: T0, to: T0 + 900n * SEC }, 1000)).toBe(2n * 60n * SEC);
    expect(tickStep({ from: T0, to: T0 + 10n * SEC }, 1000)).toBe(1n * SEC);
    expect(tickStep({ from: T0, to: T0 + MS }, 1000)).toBe(100_000n);
  });

  it("aligns ticks to the step and stays inside the view", () => {
    const v = { from: T0 + 1_234_567n, to: T0 + 3n * MS };
    const ts = ticks(v, MS);
    expect(ts.map((t) => (t - T0) / MS)).toEqual([2n, 3n]);
  });

  it("formats labels with precision matching the step", () => {
    const t = T0 + 12_345_678n;
    expect(formatTick(t, SEC)).toMatch(/^\d\d:\d\d:\d\d$/);
    expect(formatTick(t, MS)).toMatch(/^\d\d:\d\d\.012$/);
    expect(formatTick(t, 100_000n)).toMatch(/^\d\d\.012 345$/);
    expect(formatSpan(2_500_000n)).toBe("2.5 ms");
    expect(formatSpan(40n * SEC)).toBe("40 s");
  });

  it("maps pixels back to time", () => {
    const v = { from: T0, to: T0 + 10n * MS };
    expect(timeAt(250, v, 1000)).toBe(T0 + 2_500_000n);
  });
});
