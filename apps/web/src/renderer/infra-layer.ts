// Geometry of the infrastructure layer drawn by the Pixi map. Positions come from the engine
// (`TopologyView.infra`); these helpers only hit-test and place decorations.

import type { InfraChip, InfraFrame, InfraView } from "@karbird/web-engine";

/** Must match `CHIP_W` / `CHIP_H` in engine-core `infra.rs`. */
export const CHIP_W = 150;
export const CHIP_H = 28;
/** Height of a frame's title strip (engine `HEADER`). */
export const FRAME_HEADER = 44;

export const INFRA_FRAME = 0x8b5cf6;
export const INFRA_HOT = 0xef4444;
export const INFRA_NO_METRICS = 0x64748b;
export const NOISY_LINK = 0xf59e0b;

export function frameColor(f: InfraFrame): number {
  if (!f.has_metrics) return INFRA_NO_METRICS;
  return f.hot ? INFRA_HOT : INFRA_FRAME;
}

/** Accent of the selected machine (`--color-accent-2`). */
export const INFRA_SELECTED = 0x22d3ee;

export interface FrameLook {
  color: number;
  fillAlpha: number;
  strokeWidth: number;
  strokeAlpha: number;
  headerAlpha: number;
  /** Outer glow around the selected machine, in world pixels; 0 for none. */
  halo: number;
}

/** How a machine frame is drawn; the selected one stands out from every other frame. */
export function frameLook(f: InfraFrame, selected: boolean): FrameLook {
  if (selected) return { color: INFRA_SELECTED, fillAlpha: 0.12, strokeWidth: 3, strokeAlpha: 1, headerAlpha: 0.22, halo: 8 };
  return { color: frameColor(f), fillAlpha: 0.05, strokeWidth: 1.5, strokeAlpha: f.hot ? 0.9 : 0.55, headerAlpha: 0.1, halo: 0 };
}

export function chipAt(view: InfraView | null | undefined, x: number, y: number): InfraChip | null {
  return view?.chips.find((c) => Math.abs(c.x - x) <= CHIP_W / 2 && Math.abs(c.y - y) <= CHIP_H / 2) ?? null;
}

/**
 * The machine whose frame contains the point and has nothing else under it: only the
 * title strip and empty frame area select the machine, cards and chips keep their own clicks.
 */
export function frameAt(view: InfraView | null | undefined, x: number, y: number): InfraFrame | null {
  return view?.frames.find((f) => x >= f.x && x <= f.x + f.width && y >= f.y && y <= f.y + f.height) ?? null;
}

/** `×3`, with the machine count when the replicas are spread out. */
export function badgeText(count: number, hosts: number): string {
  return hosts > 1 ? `×${count} · ${hosts} machines` : `×${count}`;
}

const ARC_STEPS = 24;

/** Control point of a link's arc: bowed above the straight line, so it never lies on the call edge between the same cards. */
function arcControl(from: [number, number], to: [number, number]): [number, number] {
  const dx = to[0] - from[0];
  const dy = to[1] - from[1];
  const len = Math.hypot(dx, dy);
  if (len < 1e-6) return [from[0], from[1] - 60];
  const bow = Math.max(48, Math.min(120, len * 0.35));
  // The normal pointing up (negative y); for vertical links, to the left.
  let nx = dy / len;
  let ny = -dx / len;
  if (ny > 0 || (ny === 0 && nx > 0)) {
    nx = -nx;
    ny = -ny;
  }
  return [(from[0] + to[0]) / 2 + nx * bow * 2, (from[1] + to[1]) / 2 + ny * bow * 2];
}

/** The noisy-neighbour link as a polyline (flat x, y pairs): a quadratic arc above the cards it joins. */
export function linkArc(from: [number, number], to: [number, number]): number[] {
  const [cx, cy] = arcControl(from, to);
  const out: number[] = [];
  for (let s = 0; s <= ARC_STEPS; s++) {
    const t = s / ARC_STEPS;
    const u = 1 - t;
    out.push(u * u * from[0] + 2 * u * t * cx + t * t * to[0], u * u * from[1] + 2 * u * t * cy + t * t * to[1]);
  }
  return out;
}

/** Top of a link's arc, where its label goes. */
export function linkLabelAt(from: [number, number], to: [number, number]): [number, number] {
  const [cx, cy] = arcControl(from, to);
  return [(from[0] + 2 * cx + to[0]) / 4, (from[1] + 2 * cy + to[1]) / 4];
}
