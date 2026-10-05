// Hit-testing drawn edges: which line a click landed on. Pure, on the flat polylines the
// renderer already strokes, so what you see is what you can click.

/** Distance from (x, y) to a polyline of flat x, y pairs; Infinity for fewer than two points. */
export function distanceToPolyline(pts: readonly number[], x: number, y: number): number {
  let best = Infinity;
  for (let i = 0; i + 3 < pts.length; i += 2) {
    const ax = pts[i] ?? 0;
    const ay = pts[i + 1] ?? 0;
    const bx = pts[i + 2] ?? 0;
    const by = pts[i + 3] ?? 0;
    const dx = bx - ax;
    const dy = by - ay;
    const len2 = dx * dx + dy * dy;
    const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((x - ax) * dx + (y - ay) * dy) / len2));
    best = Math.min(best, Math.hypot(x - (ax + t * dx), y - (ay + t * dy)));
  }
  return best;
}

/** Index of the edge nearest to (x, y) within `tolerance`, or null. */
export function edgeAt(
  edges: Iterable<readonly [number, { readonly pts: readonly number[] }]>,
  x: number,
  y: number,
  tolerance: number,
): number | null {
  let best: number | null = null;
  let bestD = tolerance;
  for (const [index, path] of edges) {
    const d = distanceToPolyline(path.pts, x, y);
    if (d <= bestD) {
      best = index;
      bestD = d;
    }
  }
  return best;
}

/** Point halfway along a polyline (by length), where a test or a label can target the edge. */
export function midpoint(pts: readonly number[]): [number, number] | null {
  if (pts.length < 4) return null;
  let total = 0;
  for (let i = 0; i + 3 < pts.length; i += 2) {
    total += Math.hypot((pts[i + 2] ?? 0) - (pts[i] ?? 0), (pts[i + 3] ?? 0) - (pts[i + 1] ?? 0));
  }
  let left = total / 2;
  for (let i = 0; i + 3 < pts.length; i += 2) {
    const ax = pts[i] ?? 0;
    const ay = pts[i + 1] ?? 0;
    const seg = Math.hypot((pts[i + 2] ?? 0) - ax, (pts[i + 3] ?? 0) - ay);
    if (seg >= left && seg > 0) {
      const t = left / seg;
      return [ax + ((pts[i + 2] ?? 0) - ax) * t, ay + ((pts[i + 3] ?? 0) - ay) * t];
    }
    left -= seg;
  }
  return [pts[0] ?? 0, pts[1] ?? 0];
}
