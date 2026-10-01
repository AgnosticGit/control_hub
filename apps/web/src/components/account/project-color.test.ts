import { describe, expect, it } from "vitest";
import { projectColor } from "./project-color";

describe("projectColor", () => {
  it("is stable for the same project id", () => {
    expect(projectColor("demo")).toBe(projectColor("demo"));
  });

  it("returns a #rrggbb colour, including for an empty id", () => {
    for (const id of ["", "demo", "a-very-long-project-identifier-0123456789"]) {
      expect(projectColor(id)).toMatch(/^#[0-9a-f]{6}$/);
    }
  });

  it("spreads different projects over several colours", () => {
    const colors = new Set(Array.from({ length: 30 }, (_, i) => projectColor(`project-${i}`)));
    expect(colors.size).toBeGreaterThan(3);
  });
});
