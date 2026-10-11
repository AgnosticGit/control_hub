import { describe, expect, it } from "vitest";
import { EDGE_STYLES, build, dashes, zoomInvariantDashes, edgePath, endDirection, isEdgeStyle, pointAt, type EdgePath } from "./edge-path";

const BOX = { hw: 90, hh: 32 };

function points(p: EdgePath): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  for (let i = 0; i + 1 < p.pts.length; i += 2) out.push([p.pts[i] ?? 0, p.pts[i + 1] ?? 0]);
  return out;
}

describe("edgePath", () => {
  it("every style starts at the source card border and ends at the target card border", () => {
    for (const style of EDGE_STYLES) {
      const p = edgePath(style, 0, 0, 300, 140, BOX);
      expect(p, style).not.toBeNull();
      if (!p) continue;
      const [sx, sy] = pointAt(p, 0);
      const [ex, ey] = pointAt(p, 1);
      expect(Math.abs(sx) <= BOX.hw + 1e-6 && Math.abs(sy) <= BOX.hh + 6).toBe(true);
      expect(Math.abs(ex - 300) <= BOX.hw + 1e-6 && Math.abs(ey - 140) <= BOX.hh + 6).toBe(true);
    }
  });

  it("supports different source and target card bounds", () => {
    const target = { hw: 114, hh: 56 };
    const p = edgePath("curved", 0, 0, 300, 100, BOX, target);
    expect(p).not.toBeNull();
    expect(p?.pts.slice(-2)).toEqual([186, 62]);
  });

  it("square routes use only horizontal and vertical segments outside the rounded corners", () => {
    const p = edgePath("orthogonal", 0, 0, 400, 200, BOX);
    expect(p).not.toBeNull();
    if (!p) return;
    const pts = points(p);
    const first = pts[0];
    const second = pts[1];
    const last = pts[pts.length - 1];
    const beforeLast = pts[pts.length - 2];
    expect(first?.[1]).toBeCloseTo(second?.[1] ?? NaN);
    expect(last?.[1]).toBeCloseTo(beforeLast?.[1] ?? NaN);
    const [mx] = pointAt(p, 0.5);
    expect(mx).toBeCloseTo(200, 0);
  });

  it("metro routes cross with a 45 degree diagonal", () => {
    const p = edgePath("metro", 0, 0, 500, 100, BOX);
    expect(p).not.toBeNull();
    if (!p) return;
    const [x1, y1] = pointAt(p, 0.49);
    const [x2, y2] = pointAt(p, 0.51);
    expect(Math.abs(y2 - y1)).toBeCloseTo(Math.abs(x2 - x1), 3);
  });

  it("opposite directions between the same cards do not overlap", () => {
    for (const style of ["orthogonal", "metro"] as const) {
      const ab = edgePath(style, 0, 0, 300, 0, BOX);
      const ba = edgePath(style, 300, 0, 0, 0, BOX);
      expect(ab && ba).toBeTruthy();
      if (!ab || !ba) continue;
      expect(pointAt(ab, 0.5)[1]).not.toBeCloseTo(pointAt(ba, 0.5)[1]);
    }
  });

  it("vertical neighbours route along the y axis", () => {
    const p = edgePath("orthogonal", 0, 0, 20, 300, BOX);
    expect(p).not.toBeNull();
    if (!p) return;
    const [, sy] = pointAt(p, 0);
    const [, ey] = pointAt(p, 1);
    expect(sy).toBeCloseTo(BOX.hh);
    expect(ey).toBeCloseTo(300 - BOX.hh);
  });

  it("overlapping cards give no path", () => {
    for (const style of EDGE_STYLES) expect(edgePath(style, 0, 0, 2, 1, BOX)).toBeNull();
  });
});

describe("pointAt / endDirection / dashes", () => {
  it("keeps the dash count stable at every zoom level", () => {
    const long = build([0, 0, 1_000_000, 0]);
    const dashesAtZoomOut = zoomInvariantDashes(long);
    const dashesAtNormalZoom = zoomInvariantDashes(long);
    const dashesAtZoomIn = zoomInvariantDashes(long);
    expect(dashesAtZoomOut.length).toBe(512);
    expect(dashesAtNormalZoom.length).toBe(dashesAtZoomOut.length);
    expect(dashesAtZoomIn.length).toBe(dashesAtZoomOut.length);
    expect(dashesAtZoomIn.flat().every(Number.isFinite)).toBe(true);
  });
  const p = edgePath("orthogonal", 0, 0, 400, 0, BOX);

  it("moves by arc length", () => {
    expect(p).not.toBeNull();
    if (!p) return;
    const [x] = pointAt(p, 0.25);
    expect(x).toBeCloseTo(90 + (310 - 90) * 0.25);
    expect(pointAt(p, -1)).toEqual(pointAt(p, 0));
    expect(pointAt(p, 2)).toEqual(pointAt(p, 1));
  });

  it("points the arrow along the last segment", () => {
    if (!p) return;
    const [ux, uy] = endDirection(p);
    expect(ux).toBeCloseTo(1);
    expect(uy).toBeCloseTo(0);
  });

  it("dashes cover the path with gaps", () => {
    if (!p) return;
    const d = dashes(p, 6, 5);
    expect(d.length).toBe(Math.ceil(p.length / 11));
    for (const dash of d) expect(dash.length).toBeGreaterThanOrEqual(4);
  });

  it("validates stored style names", () => {
    expect(isEdgeStyle("metro")).toBe(true);
    expect(isEdgeStyle("zigzag")).toBe(false);
    expect(isEdgeStyle(null)).toBe(false);
  });
});
