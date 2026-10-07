import { describe, expect, it } from "vitest";
import { MACHINE_FRAME_COLORS } from "./map-colors";
import { RESOURCE_COLOR, colorAt, cssColor, serviceColors } from "./service-colors";

const MACHINE_COLORS = Object.values(MACHINE_FRAME_COLORS);

function hue(color: number): number {
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

const svc = (index: number, id: string) => ({ index, id, kind: "service" as const });

describe("serviceColors", () => {
  it("gives every service its own colour and leaves resources out", () => {
    const nodes = [svc(0, "svc_orders"), svc(1, "svc_gateway"), { index: 2, id: "res_db", kind: "database" as const }, svc(3, "svc_auth")];
    const m = serviceColors(nodes);
    expect(m.size).toBe(3);
    expect(m.has(2)).toBe(false);
    expect(new Set(m.values()).size).toBe(3);
    for (const c of m.values()) expect(c).not.toBe(RESOURCE_COLOR);
  });

  it("is stable regardless of node order or index", () => {
    const a = serviceColors([svc(0, "svc_b"), svc(1, "svc_a")]);
    const b = serviceColors([svc(7, "svc_a"), svc(9, "svc_b")]);
    expect(a.get(1)).toBe(b.get(7));
    expect(a.get(0)).toBe(b.get(9));
  });

  it("stays distinct beyond the hand-picked palette", () => {
    const colors = Array.from({ length: 40 }, (_, i) => colorAt(i));
    expect(new Set(colors).size).toBe(40);
    for (const c of colors) {
      expect(c).toBeGreaterThanOrEqual(0);
      expect(c).toBeLessThanOrEqual(0xffffff);
    }
  });

  it("keeps service hues visibly separate from machine frame states", () => {
    for (const color of Array.from({ length: 60 }, (_, i) => colorAt(i))) {
      for (const machineColor of MACHINE_COLORS) {
        expect(hueDistance(hue(color), hue(machineColor))).toBeGreaterThanOrEqual(22);
      }
    }
  });
});

describe("cssColor", () => {
  it("pads to six hex digits", () => {
    expect(cssColor(0x38bdf8)).toBe("#38bdf8");
    expect(cssColor(0x0000ff)).toBe("#0000ff");
  });
});
