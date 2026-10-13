import { build, edgePath, rounded, type CardBox, type EdgePath, type EdgeStyle } from "./edge-path";
import type { EdgeCardGroup } from "./replica-edge-pairs";

export interface EdgeObstacle {
  node: number;
  x: number;
  y: number;
  box: CardBox;
}

interface Point { x: number; y: number }
interface Rect { left: number; right: number; top: number; bottom: number }

export interface EdgeBranchPaths {
  trunk: EdgePath;
  branches: Array<{ path: EdgePath; isPrimary: boolean }>;
}

const JUNCTION_BOX: CardBox = { hw: 0, hh: 0 };

/** Build one shared route to a junction, then a directed branch into each replica card. */
export function edgeBranchPaths(
  group: EdgeCardGroup,
  style: EdgeStyle,
  boxForNode: (node: number) => CardBox,
  obstacles: readonly EdgeObstacle[] = [],
): EdgeBranchPaths | null {
  if (group.targets.length < 2) return null;
  const center = group.targets.reduce(
    (sum, { point }) => ({ x: sum.x + point.x / group.targets.length, y: sum.y + point.y / group.targets.length }),
    { x: 0, y: 0 },
  );
  const plannedJunction = { x: (group.source.x + center.x) / 2, y: (group.source.y + center.y) / 2 };
  const sourceBox = boxForNode(group.source.node);
  const trunk = obstacles.length
    ? routeBetweenCards(group.source, sourceBox, { node: -1, ...plannedJunction }, null, obstacles, style)
    : edgePath(style, group.source.x, group.source.y, plannedJunction.x, plannedJunction.y, sourceBox, JUNCTION_BOX);
  if (!trunk) return null;
  const junction = { x: trunk.pts.at(-2) ?? plannedJunction.x, y: trunk.pts.at(-1) ?? plannedJunction.y };
  const branches = group.targets.flatMap(({ point, isPrimary }) => {
    const targetBox = boxForNode(point.node);
    const routed = obstacles.length
      ? routeBetweenCards({ node: -1, ...junction }, null, point, targetBox, obstacles, style)
      : edgePath(style, junction.x, junction.y, point.x, point.y, JUNCTION_BOX, targetBox);
    const path = routed ? build([junction.x, junction.y, ...routed.pts.slice(2)]) : null;
    return path ? [{ path, isPrimary }] : [];
  });
  return branches.length > 0 ? { trunk, branches } : null;
}

const CLEARANCE = 14;

export function routeBetweenCards(
  source: { node: number } & Point,
  sourceBox: CardBox | null,
  target: { node: number } & Point,
  targetBox: CardBox | null,
  obstacles: readonly EdgeObstacle[],
  style: EdgeStyle,
): EdgePath | null {
  const blocks = obstacles
    .filter((card) => !(card.node === source.node && card.x === source.x && card.y === source.y)
      && !(card.node === target.node && card.x === target.x && card.y === target.y))
    .map((card) => ({
      left: card.x - card.box.hw - CLEARANCE,
      right: card.x + card.box.hw + CLEARANCE,
      top: card.y - card.box.hh - CLEARANCE,
      bottom: card.y + card.box.hh + CLEARANCE,
    }));
  const starts = sourceBox ? cardPorts(source, sourceBox).map((boundary) => ({ boundary, outside: moveOutside(boundary, source) })) : [{ boundary: source, outside: source }];
  const ends = targetBox ? cardPorts(target, targetBox).map((boundary) => ({ boundary, outside: moveOutside(boundary, target) })) : [{ boundary: target, outside: target }];
  let best: Point[] | null = null;
  let bestLength = Infinity;
  for (const from of starts) for (const to of ends) {
    const routed = visibilityPath(from.outside, to.outside, blocks);
    if (!routed) continue;
    const points = [from.boundary, ...routed, ...(targetBox ? [to.boundary] : [])];
    const length = points.slice(1).reduce((sum, p, i) => sum + Math.abs(p.x - (points[i]?.x ?? p.x)) + Math.abs(p.y - (points[i]?.y ?? p.y)), 0);
    if (length < bestLength) { best = points; bestLength = length; }
  }
  if (!best) return null;
  const flat = best.flatMap(({ x, y }) => [x, y]);
  const pathPoints = style === "orthogonal" ? flat : rounded(flat, 8);
  return build(pathPoints);
}

function cardPorts(point: Point, box: CardBox): Point[] {
  return [
    { x: point.x + box.hw, y: point.y },
    { x: point.x - box.hw, y: point.y },
    { x: point.x, y: point.y + box.hh },
    { x: point.x, y: point.y - box.hh },
  ];
}

function moveOutside(port: Point, center: Point): Point {
  const x = port.x === center.x ? 0 : Math.sign(port.x - center.x);
  const y = port.y === center.y ? 0 : Math.sign(port.y - center.y);
  return { x: port.x + x * CLEARANCE, y: port.y + y * CLEARANCE };
}

/** Shortest orthogonal route in a visibility grid around expanded service-card rectangles. */
function visibilityPath(start: Point, end: Point, blocks: readonly Rect[]): Point[] | null {
  if (blocks.some((rect) => inside(start, rect) || inside(end, rect))) return null;
  const xs = [...new Set([start.x, end.x, ...blocks.flatMap((r) => [r.left, r.right])])].sort((a, b) => a - b);
  const ys = [...new Set([start.y, end.y, ...blocks.flatMap((r) => [r.top, r.bottom])])].sort((a, b) => a - b);
  const points: Point[] = [];
  const indexByKey = new Map<string, number>();
  for (const y of ys) for (const x of xs) {
    const point = { x, y };
    if ((x !== start.x || y !== start.y) && (x !== end.x || y !== end.y) && blocks.some((rect) => inside(point, rect))) continue;
    indexByKey.set(key(point), points.length);
    points.push(point);
  }
  const startIndex = indexByKey.get(key(start));
  const endIndex = indexByKey.get(key(end));
  if (startIndex === undefined || endIndex === undefined) return null;
  const neighbors = points.map(() => [] as Array<{ index: number; cost: number }>);
  const connect = (a: number | undefined, b: number | undefined) => {
    if (a === undefined || b === undefined) return;
    const p = points[a]; const q = points[b];
    if (!p || !q || blocks.some((r) => crosses(p, q, r))) return;
    const cost = Math.abs(p.x - q.x) + Math.abs(p.y - q.y);
    neighbors[a]?.push({ index: b, cost }); neighbors[b]?.push({ index: a, cost });
  };
  for (const y of ys) {
    const row = xs.map((x) => indexByKey.get(key({ x, y }))).filter((v): v is number => v !== undefined);
    for (let i = 0; i + 1 < row.length; i++) connect(row[i], row[i + 1]);
  }
  for (const x of xs) {
    const col = ys.map((y) => indexByKey.get(key({ x, y }))).filter((v): v is number => v !== undefined);
    for (let i = 0; i + 1 < col.length; i++) connect(col[i], col[i + 1]);
  }
  const costs = new Array<number>(points.length).fill(Infinity);
  const previous = new Array<number>(points.length).fill(-1);
  const open: Array<{ index: number; score: number }> = [];
  heapPush(open, { index: startIndex, score: distance(points[startIndex] ?? start, end) });
  const visited = new Set<number>();
  costs[startIndex] = 0;
  while (open.length) {
    const current = heapPop(open)?.index ?? -1;
    if (current < 0 || visited.has(current)) continue;
    visited.add(current);
    if (current === endIndex) break;
    for (const next of neighbors[current] ?? []) {
      const cost = (costs[current] ?? Infinity) + next.cost;
      if (cost < (costs[next.index] ?? Infinity)) {
        costs[next.index] = cost;
        previous[next.index] = current;
        heapPush(open, { index: next.index, score: cost + distance(points[next.index] ?? end, end) });
      }
    }
  }
  if (!Number.isFinite(costs[endIndex] ?? Infinity)) return null;
  const path: Point[] = [];
  for (let cursor = endIndex; cursor >= 0; cursor = previous[cursor] ?? -1) {
    const point = points[cursor]; if (point) path.push(point);
    if (cursor === startIndex) break;
  }
  return path.reverse();
}

function inside(point: Point, rect: Rect): boolean {
  return point.x > rect.left && point.x < rect.right && point.y > rect.top && point.y < rect.bottom;
}

function crosses(a: Point, b: Point, rect: Rect): boolean {
  if (a.x === b.x) return a.x > rect.left && a.x < rect.right && Math.max(Math.min(a.y, b.y), rect.top) < Math.min(Math.max(a.y, b.y), rect.bottom);
  return a.y > rect.top && a.y < rect.bottom && Math.max(Math.min(a.x, b.x), rect.left) < Math.min(Math.max(a.x, b.x), rect.right);
}

function key(point: Point): string { return `${point.x}:${point.y}`; }

function distance(a: Point, b: Point): number { return Math.abs(a.x - b.x) + Math.abs(a.y - b.y); }

function heapPush(heap: Array<{ index: number; score: number }>, item: { index: number; score: number }): void {
  let i = heap.length;
  heap.push(item);
  while (i > 0) {
    const parent = (i - 1) >> 1;
    if ((heap[parent]?.score ?? Infinity) <= item.score) break;
    heap[i] = heap[parent] ?? item;
    i = parent;
  }
  heap[i] = item;
}

function heapPop(heap: Array<{ index: number; score: number }>): { index: number; score: number } | undefined {
  const first = heap[0];
  const last = heap.pop();
  if (!first || !last || heap.length === 0) return first;
  let i = 0;
  while (i * 2 + 1 < heap.length) {
    let child = i * 2 + 1;
    if (child + 1 < heap.length && (heap[child + 1]?.score ?? Infinity) < (heap[child]?.score ?? Infinity)) child++;
    if ((heap[child]?.score ?? Infinity) >= last.score) break;
    heap[i] = heap[child] ?? last;
    i = child;
  }
  heap[i] = last;
  return first;
}
