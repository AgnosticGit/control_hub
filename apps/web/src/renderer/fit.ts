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

export interface Camera {
  x: number;
  y: number;
  scale: number;
}

/** Zoom around a screen point, snapping back to the fitted overview at the zoom-out limit. */
export function zoomCamera(camera: Camera, factor: number, sx: number, sy: number, minimum: Camera, maxScale: number): Camera {
  const requested = camera.scale * factor;
  if (!Number.isFinite(requested) || requested <= minimum.scale) return minimum;
  const scale = Math.min(maxScale, requested);
  const worldX = (sx - camera.x) / camera.scale;
  const worldY = (sy - camera.y) / camera.scale;
  return { x: sx - worldX * scale, y: sy - worldY * scale, scale };
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
  // Too tall even at the minimum scale: show its start below the overlay rather than its middle.
  const fits = spanY * scale <= bottom - top;
  return {
    x: o.width / 2 - ((b.minX + b.maxX) / 2) * scale,
    y: fits ? (top + bottom) / 2 - ((b.minY + b.maxY) / 2) * scale : top - b.minY * scale,
    scale,
  };
}
/** Saved inventories can span hundreds of rows; fit must include their full extent. */
export const MIN_MAP_SCALE = 0.001;

/** Round an overlay outward: rounding down can put tiny overview cards underneath it. */
export function overlayInset(px: number): number {
  return Number.isFinite(px) ? Math.max(0, Math.ceil(px)) : 0;
}
