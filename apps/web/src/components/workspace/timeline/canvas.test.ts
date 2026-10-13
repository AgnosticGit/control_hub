import { describe, expect, it } from "vitest";
import { setupCanvas } from "./canvas";

function fakeCanvas() {
  let width = 0;
  let height = 0;
  let sizeWrites = 0;
  const transforms: number[][] = [];
  const context = { setTransform: (...values: number[]) => transforms.push(values) };
  const canvas = {
    get width() { return width; },
    set width(value: number) { width = value; sizeWrites += 1; },
    get height() { return height; },
    set height(value: number) { height = value; sizeWrites += 1; },
    getContext: () => context,
  } as unknown as HTMLCanvasElement;
  return { canvas, transforms, sizeWrites: () => sizeWrites };
}

describe("timeline canvas setup", () => {
  it("keeps the existing bitmap when CSS size and device scale have not changed", () => {
    const fake = fakeCanvas();
    setupCanvas(fake.canvas, 320, 80, 2);
    setupCanvas(fake.canvas, 320, 80, 2);

    expect(fake.canvas.width).toBe(640);
    expect(fake.canvas.height).toBe(160);
    expect(fake.sizeWrites()).toBe(2);
    expect(fake.transforms).toEqual([[2, 0, 0, 2, 0, 0], [2, 0, 0, 2, 0, 0]]);
  });

  it("resizes only the dimension that changed", () => {
    const fake = fakeCanvas();
    setupCanvas(fake.canvas, 320, 80, 2);
    setupCanvas(fake.canvas, 320, 96, 2);

    expect(fake.canvas.width).toBe(640);
    expect(fake.canvas.height).toBe(192);
    expect(fake.sizeWrites()).toBe(3);
  });
});
