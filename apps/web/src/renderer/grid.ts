// Background grid of the system map: world-anchored lines whose step doubles or halves with zoom,
// so the on-screen spacing stays readable. Positions are in screen pixels along one axis.

/** Base grid step in world units (layout spacing is 300 × 140). */
const GRID_BASE = 20;
export const GRID_MIN_PX = 16;
export const GRID_MAX_PX = GRID_MIN_PX * 2;
export const GRID_MAJOR_EVERY = 5;

export interface GridLine {
  pos: number;
  major: boolean;
}

/** Grid step in world units for a zoom scale. */
export function gridStep(scale: number): number {
  if (!Number.isFinite(scale) || scale <= 0) return GRID_BASE;
  let step = GRID_BASE;
  while (step * scale < GRID_MIN_PX) step *= 2;
  while (step * scale >= GRID_MAX_PX) step /= 2;
  return step;
}

/** Lines across `length` screen pixels where world 0 sits at screen `origin`. */
export function gridLines(origin: number, scale: number, length: number): GridLine[] {
  if (!Number.isFinite(origin) || !Number.isFinite(scale) || scale <= 0 || !(length > 0)) return [];
  const px = gridStep(scale) * scale;
  const out: GridLine[] = [];
  for (let i = Math.ceil(-origin / px); origin + i * px <= length; i++) {
    out.push({ pos: origin + i * px, major: i % GRID_MAJOR_EVERY === 0 });
  }
  return out;
}
