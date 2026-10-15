import { describe, expect, it } from "vitest";
import { fitCamera, MIN_MAP_SCALE, overlayInset, zoomCamera, type FitOptions } from "./fit";

const OPTS: FitOptions = { width: 1000, height: 600, padX: 100, padY: 100, topInset: 0, insetGap: 50, minScale: 0.15, maxScale: 1.3 };
const BOUNDS = { minX: 0, minY: 0, maxX: 1200, maxY: 600 };
const screenY = (y: number, c: { y: number; scale: number }) => c.y + y * c.scale;

describe("fitCamera", () => {
  it("never rounds a fractional overlay down over a tiny overview card", () => {
    expect(overlayInset(214.390625)).toBe(215);
    expect(overlayInset(-4)).toBe(0);
    expect(overlayInset(Number.NaN)).toBe(0);
  });
  it("fits a large saved inventory instead of leaving its lower cards outside the viewport", () => {
    const bounds = { minX: 0, minY: 0, maxX: 1200, maxY: 100_000 };
    const c = fitCamera(bounds, { ...OPTS, minScale: MIN_MAP_SCALE, topInset: 140 });
    expect(screenY(bounds.minY, c)).toBeGreaterThanOrEqual(140);
    expect(screenY(bounds.maxY, c)).toBeLessThanOrEqual(OPTS.height);
  });
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

  it("a map too tall even at the minimum scale starts right below the overlay instead of being centred under it", () => {
    const tall = { minX: 0, minY: 0, maxX: 1200, maxY: 20_000 };
    const c = fitCamera(tall, { ...OPTS, topInset: 200 });
    expect(c.scale).toBe(OPTS.minScale);
    expect(screenY(tall.minY, c)).toBeCloseTo(200 + 50 * c.scale);
    const plain = fitCamera(tall, OPTS);
    expect(screenY(tall.minY, plain)).toBeCloseTo(100 * plain.scale);
  });

  it("does not zoom a tiny map past the maximum scale", () => {
    const c = fitCamera({ minX: 0, minY: 0, maxX: 10, maxY: 10 }, OPTS);
    expect(c.scale).toBe(1.3);
  });
});

describe("zoomCamera", () => {
  it("allows a small zoom out past the fitted overview", () => {
    const fit = { x: 120, y: 80, scale: 0.2 };
    const firstStep = zoomCamera(fit, 0.8, 500, 300, fit, 4);
    expect(firstStep.scale).toBeCloseTo(0.16);
    const floor = zoomCamera(firstStep, 0.5, 500, 300, fit, 4);
    expect(floor.scale).toBeCloseTo(0.16);
    expect(floor).not.toEqual(fit);
  });

  it("zooms around the pointer while remaining above the fitted minimum", () => {
    const fit = { x: 0, y: 0, scale: 0.2 };
    const current = { x: 100, y: 60, scale: 0.5 };
    const next = zoomCamera(current, 0.5, 300, 200, fit, 4);
    expect(next.scale).toBe(0.25);
    expect(next.x + ((300 - current.x) / current.scale) * next.scale).toBeCloseTo(300);
    expect(next.y + ((200 - current.y) / current.scale) * next.scale).toBeCloseTo(200);
  });
});
