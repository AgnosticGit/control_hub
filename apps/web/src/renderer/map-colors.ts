// Colours of the system map, shared by the Pixi renderer and the map key so they never drift apart.

import { EXTERNAL_COLOR } from "./external";

/** Health status 0..3: no data, healthy, degraded, failing. */
export const STATUS_COLORS = [0x64748b, 0x22d3ee, 0xf59e0b, 0xef4444] as const;
export const EDGE_OK = 0x3b82f6;
export const EDGE_INFERRED = 0x475569;
export const EDGE_ERROR = 0xef4444;
export const EDGE_WARN = 0xf59e0b;
export const ACCENT = 0x22d3ee;
export const CARD_BG = 0x232528;
/** Reserved infrastructure-frame colors; service identity hues must stay perceptually separate. */
export const MACHINE_FRAME_COLORS = {
  normal: 0x8b5cf6,
  hot: 0xef4444,
  noMetrics: 0x64748b,
  selected: ACCENT,
} as const;
export { EXTERNAL_COLOR };

/** `#rrggbb` of a 0xRRGGBB colour. */
export function hex(color: number): string {
  return `#${(color & 0xffffff).toString(16).padStart(6, "0")}`;
}
