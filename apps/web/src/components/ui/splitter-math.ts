export type SplitterAxis = "row" | "column";

/** Pixels per arrow-key press. */
export const SPLITTER_KEY_STEP = 24;

export function clampSize(v: number, min: number, max: number): number {
  return Math.round(Math.max(min, Math.min(max, v)));
}

/** New size for an arrow key, or null when the key does not resize along this axis. */
export function splitterKeyValue(axis: SplitterAxis, key: string, value: number, min: number, max: number): number | null {
  const grow = axis === "row" ? "ArrowUp" : "ArrowLeft";
  const shrink = axis === "row" ? "ArrowDown" : "ArrowRight";
  if (key !== grow && key !== shrink) return null;
  return clampSize(value + (key === grow ? SPLITTER_KEY_STEP : -SPLITTER_KEY_STEP), min, max);
}

/** The panel after the handle grows as the pointer moves towards it (up or left). */
export function splitterDragValue(startValue: number, startPos: number, pos: number, min: number, max: number): number {
  return clampSize(startValue - (pos - startPos), min, max);
}
