import { describe, expect, it } from "vitest";
import { EDGE_STYLES } from "./edge-path";
import { assignLanes, portOffsets, routeEdges, type RouteEdge, type RouteNode } from "./edge-route";

const CARD_W = 172;
const CARD_H = 56;
const BOX = { hw: CARD_W / 2 + 4, hh: CARD_H / 2 + 4, portSpan: CARD_H - 20 };

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

describe("assignLanes", () => {
  it("gives overlapping intervals different lanes and reuses free ones", () => {
    const lanes = assignLanes([
      [0, 100],
      [50, 150],
      [120, 200],
    ]);
    expect(lanes[0]).not.toBe(lanes[1]);
    expect(lanes[2]).toBe(lanes[0]);
  });
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
