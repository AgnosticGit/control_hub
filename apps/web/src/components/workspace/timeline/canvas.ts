/** Resizes only when the backing bitmap changes, since assigning either dimension clears it. */
export function setupCanvas(
  canvas: HTMLCanvasElement | null,
  width: number,
  height: number,
  devicePixelRatio = window.devicePixelRatio || 1,
) {
  if (!canvas || width <= 0 || height <= 0) return null;
  const dpr = Math.min(devicePixelRatio, 2);
  const pixelWidth = Math.floor(width * dpr);
  const pixelHeight = Math.floor(height * dpr);
  if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
  if (canvas.height !== pixelHeight) canvas.height = pixelHeight;
  const ctx = canvas.getContext("2d");
  ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
  return ctx;
}
