import { describe, expect, it } from "vitest";
import { EDGE_ERROR, EDGE_INFERRED, EDGE_OK, EDGE_WARN, EXTERNAL_COLOR, STATUS_COLORS, hex } from "@/renderer/map-colors";
import { MAP_KEY, type KeyItem } from "./map-key";

const items = MAP_KEY.flatMap((s) => s.items);
const byId = (id: string): KeyItem => {
  const item = items.find((i) => i.id === id);
  if (!item) throw new Error(`no key item ${id}`);
  return item;
};

describe("hex", () => {
  it("formats 0xRRGGBB as #rrggbb with leading zeros", () => {
    expect(hex(0xef4444)).toBe("#ef4444");
    expect(hex(0x0e1729)).toBe("#0e1729");
    expect(hex(0)).toBe("#000000");
  });
});

describe("map key", () => {
  it("explains dots, lines and cards, each mark once and in words", () => {
    expect(MAP_KEY.map((s) => s.title)).toEqual([
      "Moving dots: recorded requests",
      "Lines: calls between components",
      "Cards: services and dependencies",
    ]);
    expect(new Set(items.map((i) => i.id)).size).toBe(items.length);
    for (const i of items) {
      expect(i.label.length).toBeGreaterThan(2);
      expect(i.meaning).toMatch(/\.$/);
    }
  });

  it("covers every mark the map draws", () => {
    for (const id of ["request", "wait", "response", "failed", "selected-request", "dimmed-request"]) byId(id);
    for (const id of ["observed", "inferred", "external-line", "warn-line", "error-line", "selected-line"]) byId(id);
    for (const id of ["service", "healthy", "degraded", "failing", "no-data", "external-card", "inferred-card", "selected-card"]) byId(id);
  });

  it("uses the renderer's colours", () => {
    expect(byId("observed").swatch).toMatchObject({ kind: "line", color: hex(EDGE_OK) });
    expect(byId("inferred").swatch).toMatchObject({ kind: "line", color: hex(EDGE_INFERRED), dashed: true });
    expect(byId("warn-line").swatch).toMatchObject({ color: hex(EDGE_WARN) });
    expect(byId("error-line").swatch).toMatchObject({ color: hex(EDGE_ERROR) });
    expect(byId("external-card").swatch).toMatchObject({ kind: "card", color: hex(EXTERNAL_COLOR), dashed: true });
    expect(byId("failed").swatch).toMatchObject({ kind: "dot", halo: hex(EDGE_ERROR) });
    expect(byId("response").swatch.kind).toBe("ring");
    expect(byId("degraded").swatch).toMatchObject({ glow: hex(STATUS_COLORS[2]) });
    expect(byId("failing").swatch).toMatchObject({ glow: hex(STATUS_COLORS[3]) });
    expect(byId("healthy").swatch).toMatchObject({ color: hex(STATUS_COLORS[1]) });
  });
});
