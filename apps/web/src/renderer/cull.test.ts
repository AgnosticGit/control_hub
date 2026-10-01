import { describe, expect, it } from "vitest";
import { CULL_OVERSCAN, cullRect } from "./cull";

function contains(r: { x: number; y: number; width: number; height: number }, px: number, py: number) {
  return px >= r.x && px <= r.x + r.width && py >= r.y && py <= r.y + r.height;
}

describe("cullRect", () => {
  const visible = { x: 100, y: 50, width: 800, height: 400 };

  it("keeps a card whose centre is off-screen but whose edge is visible", () => {
    const r = cullRect(visible, 90);
    expect(contains(r, visible.x - 80, visible.y + 10)).toBe(true);
    expect(contains(r, visible.x + visible.width + 80, visible.y + visible.height + 20)).toBe(true);
  });

  it("keeps cards one whole screen away on every side, so a pan back finds them already drawn", () => {
    const r = cullRect(visible, 0);
    const { x, y, width: w, height: h } = visible;
    for (const [px, py] of [
      [x - w * CULL_OVERSCAN, y + h / 2],
      [x + w + w * CULL_OVERSCAN, y + h / 2],
      [x + w / 2, y - h * CULL_OVERSCAN],
      [x + w / 2, y + h + h * CULL_OVERSCAN],
    ] as const) {
      expect(contains(r, px, py)).toBe(true);
    }
  });

  it("stays centred on the visible rectangle", () => {
    const r = cullRect(visible, 40);
    expect(r.x + r.width / 2).toBeCloseTo(visible.x + visible.width / 2);
    expect(r.y + r.height / 2).toBeCloseTo(visible.y + visible.height / 2);
  });

  it("passes an empty or broken rectangle through, which the engine treats as no culling", () => {
    expect(cullRect({ x: 0, y: 0, width: 0, height: 0 }, 90)).toEqual({ x: 0, y: 0, width: 0, height: 0 });
    const nan = { x: Number.NaN, y: 0, width: 10, height: 10 };
    expect(cullRect(nan, 90)).toBe(nan);
  });

  it("ignores a negative margin", () => {
    expect(cullRect(visible, -500)).toEqual(cullRect(visible, 0));
  });
});
