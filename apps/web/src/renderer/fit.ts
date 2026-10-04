/** World-space bounds of what the map shows (card centres, corridors, frames). */
export interface Bounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

export interface FitOptions {
  /** Screen size in pixels. */
  width: number;
  height: number;
  /** World units kept free around the bounds on each side. */
  padX: number;
  padY: number;
  /** Screen pixels at the top covered by an overlay; at most half the screen is given up. */
  topInset: number;
  /** World units kept free between the overlay and the bounds (instead of `padY`, not on top of it). */
  insetGap: number;
  minScale: number;
  maxScale: number;
}

/** Pan and zoom that fit `b`, padded, as large as allowed, below `topInset`. */
export function fitCamera(b: Bounds, o: FitOptions): { x: number; y: number; scale: number } {
  const inset = Math.min(Math.max(0, o.topInset), o.height / 2);
  const spanX = b.maxX - b.minX;
  const spanY = b.maxY - b.minY;
  let scale = Math.min(o.maxScale, o.width / (spanX + o.padX * 2), o.height / (spanY + o.padY * 2));
  if (inset > 0) scale = Math.min(scale, (o.height - inset) / (spanY + o.padY + o.insetGap));
  scale = Math.max(o.minScale, scale);
  const top = Math.max(o.padY * scale, inset + o.insetGap * scale);
  const bottom = o.height - o.padY * scale;
  return {
    x: o.width / 2 - ((b.minX + b.maxX) / 2) * scale,
    y: (top + bottom) / 2 - ((b.minY + b.maxY) / 2) * scale,
    scale,
  };
}
