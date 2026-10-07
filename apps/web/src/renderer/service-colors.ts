// One colour per service, shared by the map and the timeline, so a request dot can be traced
// back to the service it started at. Pure, no Pixi, unit-tested.

import type { EntityKindDto } from "@karbird/web-engine";
import { MACHINE_FRAME_COLORS } from "./map-colors";

const RESERVED_MACHINE_HUES = Object.values(MACHINE_FRAME_COLORS).map(rgbHue);
/** Warm, green and magenta candidates are filtered to keep a clear gap from every machine hue. */
const SERVICE_HUE_CANDIDATES = [25, 39, 53, 67, 81, 95, 109, 123, 137, 151, 290, 304, 318, 332];
const SERVICE_HUES = SERVICE_HUE_CANDIDATES.filter((hue) => RESERVED_MACHINE_HUES.every((machineHue) => hueDistance(hue, machineHue) >= 22));
const CYCLE_LIGHTNESS = [0.66, 0.55, 0.75, 0.47] as const;

/** Resources (databases, queues, external dependencies) stay neutral. */
export const RESOURCE_COLOR = 0x94a3b8;

interface ColorNode {
  index: number;
  id: string;
  kind: EntityKindDto;
}

/**
 * Node index -> colour for every service. Services are ordered by id, so the same set of
 * services always gets the same colours and no two services share one.
 */
export function serviceColors(nodes: readonly ColorNode[]): Map<number, number> {
  const services = nodes.filter((n) => n.kind === "service").sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  const out = new Map<number, number>();
  services.forEach((n, i) => out.set(n.index, colorAt(i)));
  return out;
}

/** The i-th service colour; additional palette cycles vary lightness without entering machine hues. */
export function colorAt(i: number): number {
  const cycle = Math.floor(i / SERVICE_HUES.length);
  const hue = SERVICE_HUES[i % SERVICE_HUES.length] ?? 25;
  const lightness = CYCLE_LIGHTNESS[cycle % CYCLE_LIGHTNESS.length] ?? 0.66;
  const saturation = cycle % 2 === 0 ? 0.76 : 0.68;
  return hsl(hue, saturation, lightness);
}

function rgbHue(color: number): number {
  const r = ((color >> 16) & 0xff) / 255;
  const g = ((color >> 8) & 0xff) / 255;
  const b = (color & 0xff) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const delta = max - min;
  if (delta === 0) return 0;
  if (max === r) return ((g - b) / delta * 60 + 360) % 360;
  if (max === g) return ((b - r) / delta * 60 + 120) % 360;
  return ((r - g) / delta * 60 + 240) % 360;
}

function hueDistance(a: number, b: number): number {
  const distance = Math.abs(a - b);
  return Math.min(distance, 360 - distance);
}

/** `#rrggbb` for CSS and canvas. */
export function cssColor(c: number): string {
  return `#${(c & 0xffffff).toString(16).padStart(6, "0")}`;
}

function hsl(h: number, s: number, l: number): number {
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    return Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))));
  };
  return (f(0) << 16) | (f(8) << 8) | f(4);
}
