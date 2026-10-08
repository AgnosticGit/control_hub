import { describe, expect, it } from "vitest";
import { EDGE_STYLES } from "./edge-path";
import { orderedLanes, portOffsets, routeEdges, type RouteEdge, type RouteNode } from "./edge-route";

const CARD_W = 172;
const CARD_H = 56;
const BOX = { hw: CARD_W / 2 + 4, hh: CARD_H / 2 + 4, portSpan: CARD_H - 12 };

// A shape like the demo: a fans out to b and c; a -> d skips a column through a corridor.
const nodes: RouteNode[] = [
  { index: 0, x: 0, y: 0 },
  { index: 1, x: 300, y: -60 },
  { index: 2, x: 300, y: 60 },
  { index: 3, x: 600, y: 0 },
  { index: 4, x: 600, y: 180 },
];
const edges: RouteEdge[] = [
  { index: 0, source: 0, target: 1, points: [] },
  { index: 1, source: 0, target: 2, points: [] },
  { index: 2, source: 0, target: 3, points: [[300, 150]] },
  { index: 3, source: 1, target: 3, points: [] },
  { index: 4, source: 2, target: 4, points: [] },
];

it("uses the backend corridor for backward links instead of crossing intermediate cards",()=>{
  const chain=[{index:0,x:0,y:0},{index:1,x:300,y:0},{index:2,x:600,y:0}];
  const backward:RouteEdge={index:0,source:2,target:0,points:[[750,0],[750,120],[-150,120],[-150,0]]};
  const path=routeEdges("orthogonal",chain,[backward],BOX).get(0);
  expect(path?.pts).toContain(120);
  expect(path?.pts).toEqual([600+BOX.hw,0,750,0,750,120,-150,120,-150,0,-BOX.hw,0]);
});

function insideAnyCard(x: number, y: number, except: number[]): boolean {
  return nodes.some((n) => !except.includes(n.index) && Math.abs(x - n.x) < CARD_W / 2 && Math.abs(y - n.y) < CARD_H / 2);
}

describe("portOffsets", () => {
  it("centres a single port and spreads several symmetrically", () => {
    expect(portOffsets(1, 36)).toEqual([0]);
    const o = portOffsets(3, 36);
    expect(o[0]).toBeCloseTo(-(o[2] ?? 0));
    expect(o[1]).toBe(0);
  });
});

describe("orderedLanes", () => {
  it("gives overlapping runs different lanes and lets separate ones share", () => {
    const lanes = orderedLanes([
      { y0: 0, y1: 100 },
      { y0: 50, y1: 150 },
      { y0: 300, y1: 400 },
    ]);
    expect(lanes[0]).not.toBe(lanes[1]);
    expect(lanes[2]).toBe(0);
  });

  it("going down, the run starting lower turns first; going up, the one starting higher", () => {
    expect(orderedLanes([{ y0: 0, y1: 200 }, { y0: 10, y1: 300 }])).toEqual([1, 0]);
    // The mirror image of the first case.
    expect(orderedLanes([{ y0: 0, y1: -200 }, { y0: -10, y1: -300 }])).toEqual([1, 0]);
  });
});

/** Proper crossings between two polylines (shared end points do not count). */
function crossingCount(a: number[], b: number[]): number {
  let n = 0;
  for (let i = 0; i + 3 < a.length; i += 2) {
    for (let j = 0; j + 3 < b.length; j += 2) {
      const [p1x, p1y, p2x, p2y] = [a[i] ?? 0, a[i + 1] ?? 0, a[i + 2] ?? 0, a[i + 3] ?? 0];
      const [q1x, q1y, q2x, q2y] = [b[j] ?? 0, b[j + 1] ?? 0, b[j + 2] ?? 0, b[j + 3] ?? 0];
      const d = (p2x - p1x) * (q2y - q1y) - (p2y - p1y) * (q2x - q1x);
      if (Math.abs(d) < 1e-9) continue;
      const t = ((q1x - p1x) * (q2y - q1y) - (q1y - p1y) * (q2x - q1x)) / d;
      const u = ((q1x - p1x) * (p2y - p1y) - (q1y - p1y) * (p2x - p1x)) / d;
      if (t > 1e-6 && t < 1 - 1e-6 && u > 1e-6 && u < 1 - 1e-6) n++;
    }
  }
  return n;
}

/** Shortest distance between two polylines' vertices and segments, sampled at vertices. */
function closest(a: number[], b: number[]): number {
  let best = Infinity;
  for (let i = 0; i + 1 < a.length; i += 2) {
    for (let j = 0; j + 3 < b.length; j += 2) {
      const [px, py] = [a[i] ?? 0, a[i + 1] ?? 0];
      const [x1, y1, x2, y2] = [b[j] ?? 0, b[j + 1] ?? 0, b[j + 2] ?? 0, b[j + 3] ?? 0];
      const len2 = (x2 - x1) ** 2 + (y2 - y1) ** 2;
      const t = len2 > 0 ? Math.max(0, Math.min(1, ((px - x1) * (x2 - x1) + (py - y1) * (y2 - y1)) / len2)) : 0;
      best = Math.min(best, Math.hypot(px - (x1 + t * (x2 - x1)), py - (y1 + t * (y2 - y1))));
    }
  }
  return best;
}

describe("fan-out from one card", () => {
  // Like "External clients": one card calling eight cards in the next column.
  const fanNodes: RouteNode[] = [{ index: 0, x: 0, y: 0 }, ...Array.from({ length: 8 }, (_, k) => ({ index: k + 1, x: 300, y: (k - 3.5) * 120 }))];
  const fanEdges: RouteEdge[] = Array.from({ length: 8 }, (_, k) => ({ index: k, source: 0, target: k + 1, points: [] }));

  for (const style of EDGE_STYLES) {
    it(`${style}: the edges never lie on each other`, () => {
      const paths = routeEdges(style, fanNodes, fanEdges, BOX);
      const pts = fanEdges.map((e) => paths.get(e.index)?.pts ?? []);
      for (let i = 0; i < pts.length; i++) {
        for (let j = i + 1; j < pts.length; j++) {
          // Apart by at least the line width everywhere (ports at least 4 px apart).
          expect(closest(pts[i] ?? [], pts[j] ?? []), `${style} edges ${i} and ${j}`).toBeGreaterThanOrEqual(3.5);
          if (style !== "curved") expect(crossingCount(pts[i] ?? [], pts[j] ?? []), `${style} edges ${i} and ${j} cross`).toBe(0);
        }
      }
    });
  }
});

describe("fan-out into targets far below", () => {
  // Like "External clients" in the infrastructure layout: targets mostly far below the
  // caller, at uneven distances, so the metro diagonals of neighbouring lanes run parallel.
  const ys = [-120, 120, 240, 480, 600, 840];
  const fanNodes: RouteNode[] = [{ index: 0, x: 0, y: 0 }, ...ys.map((y, k) => ({ index: k + 1, x: 300, y }))];
  const fanEdges: RouteEdge[] = ys.map((_, k) => ({ index: k, source: 0, target: k + 1, points: [] }));

  for (const style of EDGE_STYLES) {
    it(`${style}: the edges never lie on each other`, () => {
      const paths = routeEdges(style, fanNodes, fanEdges, BOX);
      const pts = fanEdges.map((e) => paths.get(e.index)?.pts ?? []);
      for (let i = 0; i < pts.length; i++) {
        for (let j = i + 1; j < pts.length; j++) {
          expect(closest(pts[i] ?? [], pts[j] ?? []), `${style} edges ${i} and ${j}`).toBeGreaterThanOrEqual(3.5);
        }
      }
    });
  }
});

describe("routeEdges", () => {
  for (const style of EDGE_STYLES) {
    it(`${style}: no route passes through a card other than its own ends`, () => {
      const paths = routeEdges(style, nodes, edges, BOX);
      expect(paths.size).toBe(edges.length);
      for (const e of edges) {
        const p = paths.get(e.index);
        expect(p).toBeDefined();
        if (!p) continue;
        for (let i = 0; i + 1 < p.pts.length; i += 2) {
          expect(insideAnyCard(p.pts[i] ?? 0, p.pts[i + 1] ?? 0, [e.source, e.target]), `${style} edge ${e.index} point ${i / 2}`).toBe(false);
        }
      }
    });
  }

  it("edges leaving one card start at different ports, ordered by their targets", () => {
    const paths = routeEdges("curved", nodes, edges, BOX);
    const startY = (i: number) => paths.get(i)?.pts[1] ?? NaN;
    // Targets: b (-60) above c (60) above the corridor (150).
    expect(startY(0)).toBeLessThan(startY(1));
    expect(startY(1)).toBeLessThan(startY(2));
  });

  it("square connectors sharing a gap turn in separate lanes", () => {
    const paths = routeEdges("orthogonal", nodes, edges, BOX);
    // In the first gap, a -> c and a -> d both turn downwards over overlapping heights.
    const verticalX = (i: number) => {
      const p = paths.get(i)?.pts ?? [];
      for (let k = 2; k + 3 < p.length; k += 2) {
        const dx = Math.abs((p[k + 2] ?? 0) - (p[k] ?? 0));
        const dy = Math.abs((p[k + 3] ?? 0) - (p[k + 1] ?? 0));
        if (dx < 1e-6 && dy > 10 && (p[k] ?? 0) < 300) return p[k] ?? NaN;
      }
      return NaN;
    };
    expect(verticalX(1)).not.toBeNaN();
    expect(verticalX(2)).not.toBeNaN();
    expect(verticalX(1)).not.toBeCloseTo(verticalX(2));
  });

  it("falls back to a direct path for edges that do not go left to right", () => {
    const back = routeEdges("curved", nodes, [{ index: 9, source: 3, target: 0, points: [] }], BOX);
    expect(back.get(9)?.length).toBeGreaterThan(0);
  });
});
