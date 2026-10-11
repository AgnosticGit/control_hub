// Routes every map edge through the layered layout from the engine. Pure, unit-tested.
//
// The engine places cards in columns and gives each edge that spans several columns a
// corridor point per intermediate column, reserved free of cards. A route therefore is:
//   source port → (across each corridor column, horizontally) → target port,
// with one connector per gap between two adjacent columns. Cards only live inside columns,
// so connectors never touch a card. To keep lines from lying on top of each other:
// - edges leaving (entering) one card use separate ports along its side, ordered by the
//   other end's height, so they do not cross at the card;
// - the vertical runs of square and metro connectors that share a gap get separate lanes.
// Edges that do not go left to right (cycles, same column) fall back to `edgePath`.

import { build, edgePath, rounded, type CardBox, type EdgePath, type EdgeStyle } from "./edge-path";

export interface RouteNode {
  index: number;
  x: number;
  y: number;
}

export interface RouteEdge {
  index: number;
  source: number;
  target: number;
  /** Corridor points from the engine, one per intermediate column. */
  points: ReadonlyArray<readonly [number, number]>;
}

export interface RouteBox extends CardBox {
  /** Vertical span on a card side over which ports are spread. */
  portSpan: number;
}

/** Distance between lanes (and between ports, at most). */
const LANE = 10;
/** Closest a lane comes to the cards on either side of its gap. */
const LANE_MARGIN = 18;
/** Vertical runs closer than this share no lane (line width plus a little air). */
const LANE_OVERLAP = 4;
/** Longest 45° diagonal of a metro connector. */
const MAX_DIAGONAL = 80;
const CURVE_STEPS = 20;

interface Connector {
  edge: number;
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/** Offsets for `count` ports spread over `span`, centred on 0. */
export function portOffsets(count: number, span: number): number[] {
  if (count <= 1) return [0];
  const step = Math.min(LANE * 1.5, span / (count - 1));
  return Array.from({ length: count }, (_, k) => (k - (count - 1) / 2) * step);
}

/**
 * Lane of each turning connector in one gap (0 = leftmost). Overlapping connectors get
 * separate lanes in an order that avoids needless crossings: of two connectors going down,
 * the one starting lower turns first; going up, the one starting higher. Connectors that do
 * not overlap share lanes.
 */
export function orderedLanes(conns: ReadonlyArray<{ y0: number; y1: number }>): number[] {
  const key = (c: { y0: number; y1: number }) => (c.y1 > c.y0 ? [0, -c.y0, -c.y1] : [1, c.y0, c.y1]);
  const order = conns.map((_, i) => i);
  order.sort((a, b) => {
    const ka = key(conns[a] ?? { y0: 0, y1: 0 });
    const kb = key(conns[b] ?? { y0: 0, y1: 0 });
    for (let k = 0; k < 3; k++) {
      const d = (ka[k] ?? 0) - (kb[k] ?? 0);
      if (d !== 0) return d;
    }
    return a - b;
  });
  const lanes = new Array<number>(conns.length).fill(0);
  // Each connector takes the lane after the last one it overlaps, so the order is kept.
  const placed: number[] = [];
  for (const i of order) {
    const c = conns[i] ?? { y0: 0, y1: 0 };
    const [lo, hi] = [Math.min(c.y0, c.y1), Math.max(c.y0, c.y1)];
    let lane = 0;
    for (const j of placed) {
      const o = conns[j] ?? { y0: 0, y1: 0 };
      if (Math.min(o.y0, o.y1) - LANE_OVERLAP <= hi && lo <= Math.max(o.y0, o.y1) + LANE_OVERLAP) lane = Math.max(lane, (lanes[j] ?? 0) + 1);
    }
    lanes[i] = lane;
    placed.push(i);
  }
  return lanes;
}

export function routeEdges(
  style: EdgeStyle,
  nodes: ReadonlyArray<RouteNode>,
  edges: ReadonlyArray<RouteEdge>,
  box: RouteBox | ((node: number) => RouteBox),
): Map<number, EdgePath> {
  const pos = new Map<number, RouteNode>();
  for (const n of nodes) pos.set(n.index, n);
  const boxFor = typeof box === "function" ? box : () => box;
  const boxes = new Map(nodes.map((node) => [node.index, boxFor(node.index)]));
  const maxBox = [...boxes.values()].reduce<RouteBox>((max, current) => ({
    hw: Math.max(max.hw, current.hw),
    hh: Math.max(max.hh, current.hh),
    portSpan: Math.max(max.portSpan, current.portSpan),
  }), { hw: 0, hh: 0, portSpan: 0 });
  const boxAt = (node: number) => boxes.get(node) ?? maxBox;
  const out = new Map<number, EdgePath>();

  // Stations of each forward edge: centre of the source, corridor points, centre of the target.
  const forward: Array<{ e: RouteEdge; stations: Array<[number, number]> }> = [];
  for (const e of edges) {
    const s = pos.get(e.source);
    const t = pos.get(e.target);
    if (!s || !t || e.source === e.target) continue;
    const sourceBox = boxAt(e.source);
    const targetBox = boxAt(e.target);
    if (t.x <= s.x) {
      const points=[s.x+sourceBox.hw,s.y,...e.points.flatMap(([x,y])=>[x,y]),t.x-targetBox.hw,t.y];
      const p = e.points.length>0 ? build(style==="orthogonal" ? points : rounded(points,12)) : edgePath(style, s.x, s.y, t.x, t.y, sourceBox, targetBox);
      if (p) out.set(e.index, p);
      continue;
    }
    forward.push({ e, stations: [[s.x, s.y], ...e.points.map(([x, y]): [number, number] => [x, y]), [t.x, t.y]] });
  }

  // Ports: order the edges on each card side by the height of the neighbouring station.
  const outPort = new Map<number, number>();
  const inPort = new Map<number, number>();
  const bySide = (key: (f: (typeof forward)[number]) => number, neighbour: (f: (typeof forward)[number]) => number, into: Map<number, number>) => {
    const groups = new Map<number, Array<(typeof forward)[number]>>();
    for (const f of forward) {
      const k = key(f);
      const g = groups.get(k) ?? [];
      g.push(f);
      groups.set(k, g);
    }
    for (const g of groups.values()) {
      g.sort((a, b) => neighbour(a) - neighbour(b) || a.e.index - b.e.index);
      const node = g[0] ? key(g[0]) : -1;
      const offs = portOffsets(g.length, boxAt(node).portSpan);
      g.forEach((f, k) => into.set(f.e.index, offs[k] ?? 0));
    }
  };
  bySide(
    (f) => f.e.source,
    (f) => f.stations[1]?.[1] ?? 0,
    outPort,
  );
  bySide(
    (f) => f.e.target,
    (f) => f.stations[f.stations.length - 2]?.[1] ?? 0,
    inPort,
  );

  // Each route: a list of horizontal "runs" through columns, joined by connectors in the gaps.
  const connectors: Connector[] = [];
  const runs = new Map<number, Array<[number, number, number, number]>>();
  for (const { e, stations } of forward) {
    const list: Array<[number, number, number, number]> = [];
    const last = stations.length - 1;
    const sourceBox = boxAt(e.source);
    const targetBox = boxAt(e.target);
    stations.forEach(([x, y], k) => {
      if (k === 0) {
        const py = y + (outPort.get(e.index) ?? 0);
        list.push([x + sourceBox.hw, py, x + sourceBox.hw, py]);
      } else if (k === last) {
        const py = y + (inPort.get(e.index) ?? 0);
        list.push([x - targetBox.hw, py, x - targetBox.hw, py]);
      } else {
        list.push([x - maxBox.hw, y, x + maxBox.hw, y]);
      }
    });
    runs.set(e.index, list);
    for (let k = 0; k + 1 < list.length; k++) {
      const a = list[k];
      const b = list[k + 1];
      if (a && b) connectors.push({ edge: e.index, x0: a[2], y0: a[3], x1: b[0], y1: b[1] });
    }
  }

  // Lanes: square and metro connectors that turn in the same gap get separate vertical runs.
  const laneX = new Map<Connector, number>();
  const diagonal = new Map<Connector, number>();
  const gaps = new Map<string, Connector[]>();
  for (const c of connectors) {
    const key = `${Math.round(c.x0)}:${Math.round(c.x1)}`;
    const g = gaps.get(key) ?? [];
    g.push(c);
    gaps.set(key, g);
  }
  for (const g of gaps.values()) {
    const turning = g.filter((c) => Math.abs(c.y1 - c.y0) >= 0.5);
    const lanes = orderedLanes(turning);
    const count = lanes.reduce((m, l) => Math.max(m, l + 1), 0);
    const first = turning[0];
    if (!first) continue;
    // Lanes keep clear of the cards on both sides of the gap.
    const room = Math.max(0, first.x1 - first.x0 - 2 * LANE_MARGIN);
    const step = count > 1 ? Math.min(LANE, room / (count - 1)) : 0;
    turning.forEach((c, i) => {
      const mid = (c.x0 + c.x1) / 2;
      laneX.set(c, mid + ((lanes[i] ?? 0) - (count - 1) / 2) * step);
    });
    // One diagonal length per gap: with the lane order above, equal 45° diagonals of
    // overlapping connectors are parallel and never collinear.
    let diag = MAX_DIAGONAL;
    for (const c of turning) {
      const lx = laneX.get(c) ?? (c.x0 + c.x1) / 2;
      diag = Math.min(diag, Math.max(0, Math.min(lx - c.x0, c.x1 - lx) * 2 - 8));
    }
    for (const c of turning) diagonal.set(c, diag);
  }

  const byEdge = new Map<number, Connector[]>();
  for (const c of connectors) {
    const list = byEdge.get(c.edge) ?? [];
    list.push(c);
    byEdge.set(c.edge, list);
  }
  for (const { e } of forward) {
    const list = runs.get(e.index) ?? [];
    const conns = byEdge.get(e.index) ?? [];
    const corners: number[] = [];
    const pts: number[] = [];
    const first = list[0];
    if (!first) continue;
    // Square and metro join everything into one corner list rounded at the end; curves are
    // sampled per connector.
    if (style === "curved") {
      pts.push(first[0], first[1]);
      list.forEach((run, k) => {
        if (k > 0) {
          const c = conns[k - 1];
          if (c) pts.push(...bezier(c));
        }
        pts.push(run[2], run[3]);
      });
    } else {
      corners.push(first[0], first[1]);
      list.forEach((run, k) => {
        if (k > 0) {
          const c = conns[k - 1];
          if (c) corners.push(...turn(style, c, laneX.get(c) ?? (c.x0 + c.x1) / 2, diagonal.get(c) ?? 0));
          corners.push(run[0], run[1]);
        }
        corners.push(run[2], run[3]);
      });
      pts.push(...rounded(dedupe(corners), style === "metro" ? 10 : 12));
    }
    out.set(e.index, build(pts));
  }
  return out;
}

/** S-curve with horizontal tangents from (x0, y0) to (x1, y1), end points excluded. */
function bezier(c: Connector): number[] {
  const dx = (c.x1 - c.x0) / 2;
  const out: number[] = [];
  for (let s = 1; s < CURVE_STEPS; s++) {
    const t = s / CURVE_STEPS;
    const u = 1 - t;
    const x = u * u * u * c.x0 + 3 * u * u * t * (c.x0 + dx) + 3 * u * t * t * (c.x1 - dx) + t * t * t * c.x1;
    const y = u * u * u * c.y0 + 3 * u * u * t * c.y0 + 3 * u * t * t * c.y1 + t * t * t * c.y1;
    out.push(x, y);
  }
  return out;
}

/** Corner points of a square or metro connector turning at `lx`, end points excluded. */
function turn(style: EdgeStyle, c: Connector, lx: number, maxDiag: number): number[] {
  const dy = c.y1 - c.y0;
  if (Math.abs(dy) < 0.5) return [];
  if (style !== "metro") return [lx, c.y0, lx, c.y1];
  // 45° diagonal centred on the lane, `maxDiag` long at most; the rest is vertical.
  const diag = Math.min(Math.abs(dy), maxDiag);
  const side = dy > 0 ? 1 : -1;
  const x0 = lx - diag / 2;
  const x1 = lx + diag / 2;
  return [x0, c.y0, x1, c.y0 + side * diag, x1, c.y1];
}

/** Drops consecutive duplicate points so corner rounding sees real corners only. */
function dedupe(pts: number[]): number[] {
  const out: number[] = [];
  for (let i = 0; i + 1 < pts.length; i += 2) {
    const x = pts[i] ?? 0;
    const y = pts[i + 1] ?? 0;
    const n = out.length;
    if (n >= 2 && Math.abs((out[n - 2] ?? 0) - x) < 1e-6 && Math.abs((out[n - 1] ?? 0) - y) < 1e-6) continue;
    out.push(x, y);
  }
  return out;
}
