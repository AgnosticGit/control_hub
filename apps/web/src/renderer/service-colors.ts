// One colour per service, shared by the map and the timeline, so a request dot can be traced
// back to the service it started at. Pure, no Pixi, unit-tested.

import type { EntityKindDto } from "@controlhub/web-engine";

/** Hand-picked, well separated on the dark background; red is left for errors. */
const PALETTE = [
  0x38bdf8, // sky
  0xa78bfa, // violet
  0x34d399, // emerald
  0xf472b6, // pink
  0xfacc15, // yellow
  0x818cf8, // indigo
  0xfb923c, // orange
  0x2dd4bf, // teal
  0xa3e635, // lime
  0xe879f9, // fuchsia
  0x93c5fd, // pale blue
  0xfde68a, // sand
];

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

/** The i-th colour: the palette first, then golden-angle hues. */
export function colorAt(i: number): number {
  const fixed = PALETTE[i];
  if (fixed !== undefined) return fixed;
  return hsl((((i - PALETTE.length) * 137.508 + 20) % 360 + 360) % 360, 0.7, 0.66);
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
