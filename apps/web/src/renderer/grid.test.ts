import { describe, expect, it } from "vitest";
import { GRID_MAJOR_EVERY, GRID_MAX_PX, GRID_MIN_PX, gridLines, gridStep } from "./grid";

describe("gridStep", () => {
  it("keeps the on-screen spacing between the min and max at every zoom", () => {
    for (const scale of [0.15, 0.3, 0.5, 1, 1.3, 2, 4]) {
      const px = gridStep(scale) * scale;
      expect(px).toBeGreaterThanOrEqual(GRID_MIN_PX);
      expect(px).toBeLessThan(GRID_MAX_PX);
    }
  });

  it("falls back to a sane step for a broken scale", () => {
    expect(gridStep(0)).toBeGreaterThan(0);
    expect(gridStep(Number.NaN)).toBeGreaterThan(0);
  });
});

describe("gridLines", () => {
  it("covers the whole screen length with evenly spaced lines", () => {
    const pos = gridLines(13, 1, 400).map((l) => l.pos);
    expect(pos.length).toBeGreaterThan(5);
    expect(pos[0]).toBeGreaterThanOrEqual(0);
    expect(pos[0]).toBeLessThan(gridStep(1));
    expect(pos.at(-1)).toBeLessThanOrEqual(400);
    const gaps = pos.slice(1).map((p, i) => p - (pos[i] ?? 0));
    for (const g of gaps) expect(g).toBeCloseTo(gridStep(1));
  });

  it("stays anchored to the world: panning moves lines by the same amount", () => {
    const [a] = gridLines(0, 1, 400);
    const [b] = gridLines(7, 1, 400);
    expect(b?.pos).toBeCloseTo(((a?.pos ?? 0) + 7) % gridStep(1));
  });

  it("marks every fifth line as major, starting at the world origin", () => {
    const lines = gridLines(100, 1, 1000);
    const origin = lines.find((l) => Math.abs(l.pos - 100) < 1e-6);
    expect(origin?.major).toBe(true);
    const majors = lines.filter((l) => l.major);
    const minors = lines.filter((l) => !l.major);
    expect(majors.length).toBeGreaterThan(0);
    expect(minors.length).toBeGreaterThan(majors.length);
    const step = gridStep(1);
    for (const m of majors) expect(Math.abs(Math.round((m.pos - 100) / step) % GRID_MAJOR_EVERY)).toBe(0);
  });

  it("draws nothing for an empty or broken viewport", () => {
    expect(gridLines(0, 1, 0)).toEqual([]);
    expect(gridLines(Number.NaN, 1, 400)).toEqual([]);
    expect(gridLines(0, 0, 400)).toEqual([]);
  });
});
