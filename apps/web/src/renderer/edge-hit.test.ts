import { describe, expect, it } from "vitest";
import { distanceToPolyline, edgeAt, midpoint } from "./edge-hit";

describe("distanceToPolyline", () => {
  it("measures to the nearest segment, clamped to its ends", () => {
    const l = [0, 0, 100, 0, 100, 100];
    expect(distanceToPolyline(l, 50, 5)).toBe(5);
    expect(distanceToPolyline(l, 105, 50)).toBe(5);
    expect(distanceToPolyline(l, -3, -4)).toBe(5);
  });

  it("handles degenerate input", () => {
    expect(distanceToPolyline([], 0, 0)).toBe(Infinity);
    expect(distanceToPolyline([1, 1], 0, 0)).toBe(Infinity);
    expect(distanceToPolyline([0, 0, 0, 0], 3, 4)).toBe(5);
  });
});

describe("edgeAt", () => {
  const edges = new Map([
    [3, { pts: [0, 0, 100, 0] }],
    [7, { pts: [0, 10, 100, 10] }],
  ]);

  it("picks the nearest edge within the tolerance", () => {
    expect(edgeAt(edges, 50, 2, 6)).toBe(3);
    expect(edgeAt(edges, 50, 8, 6)).toBe(7);
  });

  it("misses when every edge is farther than the tolerance", () => {
    expect(edgeAt(edges, 50, 40, 6)).toBeNull();
    expect(edgeAt(new Map(), 0, 0, 6)).toBeNull();
  });
});

describe("midpoint", () => {
  it("is halfway along the length, not the middle vertex", () => {
    expect(midpoint([0, 0, 100, 0])).toEqual([50, 0]);
    expect(midpoint([0, 0, 30, 0, 30, 10])).toEqual([20, 0]);
    expect(midpoint([0, 0, 0, 100, 100, 100])).toEqual([0, 100]);
  });

  it("is null without a segment", () => {
    expect(midpoint([5, 5])).toBeNull();
  });
});
