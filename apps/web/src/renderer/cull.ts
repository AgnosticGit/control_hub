/** Screens of map kept drawn on each side of the visible one, so panning never reveals a missing card. */
export const CULL_OVERSCAN = 1;

export interface WorldRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * The world-space rectangle the engine culls against: the visible rectangle grown by
 * `CULL_OVERSCAN` screens on each side plus `margin` (the card half-size, since the engine
 * tests node centres).
 */
export function cullRect(visible: WorldRect, margin: number): WorldRect {
  const { x, y, width, height } = visible;
  if (![x, y, width, height, margin].every(Number.isFinite) || width <= 0 || height <= 0) return visible;
  const padX = width * CULL_OVERSCAN + Math.max(0, margin);
  const padY = height * CULL_OVERSCAN + Math.max(0, margin);
  return { x: x - padX, y: y - padY, width: width + padX * 2, height: height + padY * 2 };
}
