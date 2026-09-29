import { describe, expect, it } from "vitest";
import { SPLITTER_KEY_STEP, clampSize, splitterDragValue, splitterKeyValue } from "./splitter-math";

describe("splitterKeyValue", () => {
  it("row handle: ArrowUp grows, ArrowDown shrinks the panel below", () => {
    expect(splitterKeyValue("row", "ArrowUp", 200, 100, 400)).toBe(200 + SPLITTER_KEY_STEP);
    expect(splitterKeyValue("row", "ArrowDown", 200, 100, 400)).toBe(200 - SPLITTER_KEY_STEP);
  });

  it("column handle: ArrowLeft grows, ArrowRight shrinks the panel to the right", () => {
    expect(splitterKeyValue("column", "ArrowLeft", 300, 100, 400)).toBe(324);
    expect(splitterKeyValue("column", "ArrowRight", 300, 100, 400)).toBe(276);
  });

  it("ignores keys of the other axis and unrelated keys", () => {
    expect(splitterKeyValue("row", "ArrowLeft", 200, 0, 400)).toBeNull();
    expect(splitterKeyValue("column", "ArrowUp", 200, 0, 400)).toBeNull();
    expect(splitterKeyValue("row", "Enter", 200, 0, 400)).toBeNull();
  });

  it("stays within bounds", () => {
    expect(splitterKeyValue("row", "ArrowUp", 390, 100, 400)).toBe(400);
    expect(splitterKeyValue("row", "ArrowDown", 110, 100, 400)).toBe(100);
  });
});

describe("splitterDragValue", () => {
  it("grows as the pointer moves towards the panel and clamps", () => {
    expect(splitterDragValue(200, 500, 450, 100, 400)).toBe(250);
    expect(splitterDragValue(200, 500, 560, 100, 400)).toBe(140);
    expect(splitterDragValue(200, 500, 0, 100, 400)).toBe(400);
    expect(splitterDragValue(200, 500, 1000, 100, 400)).toBe(100);
  });

  it("rounds to whole pixels", () => {
    expect(splitterDragValue(200, 500, 499.4, 0, 400)).toBe(201);
    expect(clampSize(10.5, 0, 100)).toBe(11);
  });
});
