import { describe, expect, it } from "vitest";
import { fitCamera, type FitOptions } from "./fit";

const OPTS: FitOptions = { width: 1000, height: 600, padX: 100, padY: 100, topInset: 0, insetGap: 50, minScale: 0.15, maxScale: 1.3 };
const BOUNDS = { minX: 0, minY: 0, maxX: 1200, maxY: 600 };
const screenY = (y: number, c: { y: number; scale: number }) => c.y + y * c.scale;

describe("fitCamera", () => {
  it("centres the padded bounds on the screen", () => {
    const c = fitCamera(BOUNDS, OPTS);
    expect(c.scale).toBeCloseTo(Math.min(1000 / 1400, 600 / 800));
    expect(c.x + 600 * c.scale).toBeCloseTo(500);
    expect(screenY(300, c)).toBeCloseTo(300);
  });

  it("keeps the content and a gap below a top overlay, and the bottom padding", () => {
    const c = fitCamera(BOUNDS, { ...OPTS, topInset: 140 });
    expect(screenY(BOUNDS.minY, c)).toBeGreaterThanOrEqual(140 + 50 * c.scale - 1e-6);
    expect(screenY(BOUNDS.maxY, c)).toBeLessThanOrEqual(600 - 100 * c.scale + 1e-6);
  });

  it("an overlay inside the top padding costs no zoom", () => {
    const plain = fitCamera(BOUNDS, OPTS);
    const c = fitCamera(BOUNDS, { ...OPTS, topInset: 20 });
    expect(c.scale).toBeCloseTo(plain.scale);
    expect(c.y).toBeCloseTo(plain.y);
  });

  it("the overlay replaces the top padding instead of adding to it", () => {
    const c = fitCamera(BOUNDS, { ...OPTS, topInset: 140 });
    expect(c.scale).toBeCloseTo((600 - 140) / (600 + 100 + 50));
  });

  it("never gives up more than half the screen to an overlay", () => {
    const c = fitCamera(BOUNDS, { ...OPTS, topInset: 5000 });
    expect(screenY(BOUNDS.minY, c)).toBeGreaterThanOrEqual(300);
    expect(c.scale).toBeGreaterThanOrEqual(OPTS.minScale);
  });

  it("does not zoom a tiny map past the maximum scale", () => {
    const c = fitCamera({ minX: 0, minY: 0, maxX: 10, maxY: 10 }, OPTS);
    expect(c.scale).toBe(1.3);
  });
});
