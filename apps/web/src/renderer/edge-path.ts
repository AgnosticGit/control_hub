// Edge routes between node cards as polylines. Pure functions, no Pixi, so they are
// unit-tested. Dots move by arc length, so a hop's progress means the same distance along
// the line whatever the style.

export type EdgeStyle = "curved" | "orthogonal" | "metro";
export const EDGE_STYLES: readonly EdgeStyle[] = ["curved", "orthogonal", "metro"];
export const DEFAULT_EDGE_STYLE: EdgeStyle = "curved";

export function isEdgeStyle(v: unknown): v is EdgeStyle {
  return typeof v === "string" && (EDGE_STYLES as readonly string[]).includes(v);
}

/** Half size of a card plus the gap between the card border and the line end. */
export interface CardBox {
  hw: number;
  hh: number;
}

export interface EdgePath {
  /** Flat x, y pairs; at least two points. */
  pts: number[];
  /** Cumulative length at each point; `cum[0] = 0`. */
  cum: number[];
  length: number;
}

/** Opposite directions between the same cards get lanes this far apart. */
const LANE = 5;
const CURVE_STEPS = 24;
const CORNER_STEPS = 6;
const MIN_LENGTH = 8;

export function edgePath(style: EdgeStyle, sx: number, sy: number, tx: number, ty: number, box: CardBox): EdgePath | null {
  const pts = style === "curved" ? curvedPoints(sx, sy, tx, ty, box) : routedPoints(style, sx, sy, tx, ty, box);
  if (!pts) return null;
  const path = build(pts);
  return path.length < MIN_LENGTH ? null : path;
}

/** Point at fraction `t` (0..1) of the path length. */
export function pointAt(p: EdgePath, t: number): [number, number] {
  const d = Math.max(0, Math.min(1, t)) * p.length;
  let lo = 0;
  let hi = p.cum.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if ((p.cum[mid] ?? 0) <= d) lo = mid;
    else hi = mid;
  }
  const c0 = p.cum[lo] ?? 0;
  const seg = (p.cum[hi] ?? 0) - c0;
  const k = seg > 0 ? (d - c0) / seg : 0;
  const x0 = p.pts[lo * 2] ?? 0;
  const y0 = p.pts[lo * 2 + 1] ?? 0;
  return [x0 + ((p.pts[hi * 2] ?? 0) - x0) * k, y0 + ((p.pts[hi * 2 + 1] ?? 0) - y0) * k];
}

/** Unit direction of the path at its end, for the arrow head. */
export function endDirection(p: EdgePath): [number, number] {
  const n = p.pts.length / 2;
  const bx = p.pts[(n - 1) * 2] ?? 0;
  const by = p.pts[(n - 1) * 2 + 1] ?? 0;
  const [ax, ay] = pointAt(p, Math.max(0, 1 - 6 / p.length));
  const len = Math.hypot(bx - ax, by - ay);
  return len > 0 ? [(bx - ax) / len, (by - ay) / len] : [1, 0];
}

/** Dash polylines (flat x, y pairs) along the path. */
export function dashes(p: EdgePath, on: number, off: number): number[][] {
  const out: number[][] = [];
  for (let d = 0; d < p.length; d += on + off) {
    const end = Math.min(p.length, d + on);
    const dash = [...pointAt(p, d / p.length)];
    for (let i = 1; i < p.cum.length - 1; i++) {
      const c = p.cum[i] ?? 0;
      if (c > d && c < end) dash.push(p.pts[i * 2] ?? 0, p.pts[i * 2 + 1] ?? 0);
    }
    dash.push(...pointAt(p, end / p.length));
    out.push(dash);
  }
  return out;
}

/** Dash in map coordinates so zooming never changes a route's dash count. */
export function zoomInvariantDashes(p: EdgePath): number[][] {
  // Bound drawing work for long saved server corridors independently of camera scale.
  const step = Math.max(1, p.length / (11 * 512));
  return dashes(p, 6 * step, 5 * step);
}

/** A path through the given points (flat x, y pairs); duplicate points are dropped. */
export function build(pts: number[]): EdgePath {
  const clean: number[] = [pts[0] ?? 0, pts[1] ?? 0];
  const cum = [0];
  let length = 0;
  for (let i = 2; i + 1 < pts.length; i += 2) {
    const x = pts[i] ?? 0;
    const y = pts[i + 1] ?? 0;
    const step = Math.hypot(x - (clean[clean.length - 2] ?? 0), y - (clean[clean.length - 1] ?? 0));
    if (step < 1e-6) continue;
    length += step;
    clean.push(x, y);
    cum.push(length);
  }
  if (clean.length === 2) {
    clean.push(clean[0] ?? 0, clean[1] ?? 0);
    cum.push(0);
  }
  return { pts: clean, cum, length };
}

/** Point where the ray from the card centre towards (tx, ty) leaves the card. */
function cardExit(cx: number, cy: number, tx: number, ty: number, box: CardBox): [number, number] {
  const dx = tx - cx;
  const dy = ty - cy;
  const sx = dx === 0 ? Infinity : box.hw / Math.abs(dx);
  const sy = dy === 0 ? Infinity : box.hh / Math.abs(dy);
  const s = Math.min(sx, sy, 1);
  return [cx + dx * s, cy + dy * s];
}

/** Gently bowed quadratic curve; opposite directions bow to opposite sides. */
function curvedPoints(sx: number, sy: number, tx: number, ty: number, box: CardBox): number[] | null {
  const [ax, ay] = cardExit(sx, sy, tx, ty, box);
  const [bx, by] = cardExit(tx, ty, sx, sy, box);
  const len = Math.hypot(bx - ax, by - ay);
  if (len < MIN_LENGTH) return null;
  const bow = Math.min(40, len * 0.12);
  const cx = (ax + bx) / 2 + (-(by - ay) / len) * bow;
  const cy = (ay + by) / 2 + ((bx - ax) / len) * bow;
  const out: number[] = [];
  for (let s = 0; s <= CURVE_STEPS; s++) {
    const t = s / CURVE_STEPS;
    const u = 1 - t;
    out.push(u * u * ax + 2 * u * t * cx + t * t * bx, u * u * ay + 2 * u * t * cy + t * t * by);
  }
  return out;
}

/**
 * Square (right angles) or metro (45° diagonal) route. The line leaves the card side facing
 * the target along the dominant axis, turns once in the middle and enters the target from
 * the opposite side.
 */
function routedPoints(style: EdgeStyle, sx: number, sy: number, tx: number, ty: number, box: CardBox): number[] | null {
  const gapX = Math.abs(tx - sx) - 2 * box.hw;
  const gapY = Math.abs(ty - sy) - 2 * box.hh;
  if (gapX < 0 && gapY < 0) return null;
  // Route along the axis with the wider free gap between the two cards.
  const horizontal = gapX >= gapY;
  // Work in a frame where the main axis is x; swap back at the end.
  const [s0, s1, t0, t1, half] = horizontal ? [sx, sy, tx, ty, box.hw] : [sy, sx, ty, tx, box.hh];
  const dir = t0 >= s0 ? 1 : -1;
  const lane = horizontal ? dir * LANE : -dir * LANE;
  const a0 = s0 + dir * half;
  const b0 = t0 - dir * half;
  const a1 = s1 + lane;
  const b1 = t1 + lane;
  const mid = (a0 + b0) / 2;
  const cross = Math.abs(b1 - a1);
  let main: number[];
  if (cross < 0.5) {
    main = [a0, a1, b0, b1];
  } else if (style === "metro") {
    // 45° diagonal as long as the run allows; the rest of the cross distance is vertical.
    const diag = Math.min(cross, Math.abs(b0 - a0) * 0.7);
    const side = b1 >= a1 ? 1 : -1;
    const x0 = mid - (dir * diag) / 2;
    const x1 = mid + (dir * diag) / 2;
    main = [a0, a1, x0, a1, x1, a1 + side * diag, x1, b1, b0, b1];
  } else {
    main = [a0, a1, mid, a1, mid, b1, b0, b1];
  }
  const pts = rounded(main, style === "metro" ? 10 : 14);
  if (horizontal) return pts;
  const swapped: number[] = [];
  for (let i = 0; i + 1 < pts.length; i += 2) swapped.push(pts[i + 1] ?? 0, pts[i] ?? 0);
  return swapped;
}

/** Replaces each interior corner with a short quadratic arc of radius up to `r`. */
export function rounded(pts: number[], r: number): number[] {
  const n = pts.length / 2;
  if (n < 3) return pts;
  const at = (i: number): [number, number] => [pts[i * 2] ?? 0, pts[i * 2 + 1] ?? 0];
  const out: number[] = [...at(0)];
  for (let i = 1; i < n - 1; i++) {
    const [px, py] = at(i - 1);
    const [cx, cy] = at(i);
    const [nx, ny] = at(i + 1);
    const l1 = Math.hypot(cx - px, cy - py);
    const l2 = Math.hypot(nx - cx, ny - cy);
    const rr = Math.min(r, l1 / 2, l2 / 2);
    if (rr < 0.5) {
      out.push(cx, cy);
      continue;
    }
    const x0 = cx - ((cx - px) / l1) * rr;
    const y0 = cy - ((cy - py) / l1) * rr;
    const x1 = cx + ((nx - cx) / l2) * rr;
    const y1 = cy + ((ny - cy) / l2) * rr;
    for (let s = 0; s <= CORNER_STEPS; s++) {
      const t = s / CORNER_STEPS;
      const u = 1 - t;
      out.push(u * u * x0 + 2 * u * t * cx + t * t * x1, u * u * y0 + 2 * u * t * cy + t * t * y1);
    }
  }
  out.push(...at(n - 1));
  return out;
}
