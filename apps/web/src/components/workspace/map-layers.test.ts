import { describe, expect, it } from "vitest";
import { DEFAULT_MAP_LAYERS, needsInfrastructureSnapshot, parseMapLayers } from "./map-layers";

describe("map layer preferences", () => {
  it("defaults every optional layer to off", () => {
    expect(DEFAULT_MAP_LAYERS).toEqual({ infrastructure: false, onlyProblems: false, hideInferred: false, tableView: false, serviceMetrics: false });
  });

  it("loads legacy preferences and defaults the newly introduced layer off", () => {
    expect(parseMapLayers(JSON.stringify({ infrastructure: true, onlyProblems: false, hideInferred: true, tableView: false }))).toEqual({
      infrastructure: true, onlyProblems: false, hideInferred: true, tableView: false, serviceMetrics: false,
    });
  });

  it("round-trips the service metrics layer preference", () => {
    expect(parseMapLayers(JSON.stringify({ ...DEFAULT_MAP_LAYERS, serviceMetrics: true }))).toEqual({ ...DEFAULT_MAP_LAYERS, serviceMetrics: true });
  });

  it("rejects malformed existing preferences instead of silently resetting them", () => {
    expect(parseMapLayers(JSON.stringify({ ...DEFAULT_MAP_LAYERS, infrastructure: "yes" }))).toBeNull();
    expect(parseMapLayers("not json")).toBeNull();
  });
});

describe("infrastructure snapshot requirements", () => {
  it("keeps the shared snapshot loaded for replica counts when optional layers are off", () => {
    expect(needsInfrastructureSnapshot()).toBe(true);
  });
});
