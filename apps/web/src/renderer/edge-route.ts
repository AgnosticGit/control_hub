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
 * Lane index for each vertical interval so that overlapping intervals (with a margin) never
 * share a lane: greedy interval colouring in order of the interval start.
 */
export function assignLanes(intervals: ReadonlyArray<readonly [number, number]>, margin = 4): number[] {
  const order = intervals.map((_, i) => i).sort((a, b) => (intervals[a]?.[0] ?? 0) - (intervals[b]?.[0] ?? 0) || a - b);
  const laneEnd: number[] = [];
  const lanes = new Array<number>(intervals.length).fill(0);
  for (const i of order) {
    const [lo, hi] = intervals[i] ?? [0, 0];
    let lane = laneEnd.findIndex((end) => end + margin <= lo);
    if (lane < 0) {
      lane = laneEnd.length;
      laneEnd.push(hi);
    } else {
      laneEnd[lane] = hi;
    }
    lanes[i] = lane;
  }
  return lanes;
}

export function routeEdges(
  style: EdgeStyle,
  nodes: ReadonlyArray<RouteNode>,
  edges: ReadonlyArray<RouteEdge>,
  box: RouteBox,
): Map<number, EdgePath> {
  const pos = new Map<number, RouteNode>();
  for (const n of nodes) pos.set(n.index, n);
  const out = new Map<number, EdgePath>();

  // Stations of each forward edge: centre of the source, corridor points, centre of the target.
  const forward: Array<{ e: RouteEdge; stations: Array<[number, number]> }> = [];
  for (const e of edges) {
    const s = pos.get(e.source);
    const t = pos.get(e.target);
    if (!s || !t || e.source === e.target) continue;
    if (t.x <= s.x) {
      const p = edgePath(style, s.x, s.y, t.x, t.y, box);
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
      const offs = portOffsets(g.length, box.portSpan);
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
    stations.forEach(([x, y], k) => {
      if (k === 0) {
        const py = y + (outPort.get(e.index) ?? 0);
        list.push([x + box.hw, py, x + box.hw, py]);
      } else if (k === last) {
        const py = y + (inPort.get(e.index) ?? 0);
        list.push([x - box.hw, py, x - box.hw, py]);
      } else {
        list.push([x - box.hw, y, x + box.hw, y]);
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
  const gaps = new Map<string, Connector[]>();
  for (const c of connectors) {
    const key = `${Math.round(c.x0)}:${Math.round(c.x1)}`;
    const g = gaps.get(key) ?? [];
    g.push(c);
    gaps.set(key, g);
  }
  for (const g of gaps.values()) {
    const turning = g.filter((c) => Math.abs(c.y1 - c.y0) >= 0.5);
    const lanes = assignLanes(turning.map((c) => [Math.min(c.y0, c.y1), Math.max(c.y0, c.y1)] as const));
    const count = lanes.reduce((m, l) => Math.max(m, l + 1), 0);
    turning.forEach((c, i) => {
      const mid = (c.x0 + c.x1) / 2;
      laneX.set(c, mid + ((lanes[i] ?? 0) - (count - 1) / 2) * LANE);
    });
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
          if (c) corners.push(...turn(style, c, laneX.get(c) ?? (c.x0 + c.x1) / 2));
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
function turn(style: EdgeStyle, c: Connector, lx: number): number[] {
  const dy = c.y1 - c.y0;
  if (Math.abs(dy) < 0.5) return [];
  if (style !== "metro") return [lx, c.y0, lx, c.y1];
  // 45° diagonal centred on the lane, as long as the gap allows; the rest is vertical.
  const room = Math.max(0, Math.min(lx - c.x0, c.x1 - lx) * 2 - 8);
  const diag = Math.min(Math.abs(dy), room);
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
