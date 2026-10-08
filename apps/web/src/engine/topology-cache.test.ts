import { describe, expect, it } from "vitest";
import { applyTopologyResponse, topologyCacheKey } from "./topology-cache";

describe("topology conditional request identity", () => {
  it("reuses a live graph while its rolling time range advances", () => {
    expect(topologyCacheKey("/api/v1/projects/p/environments/e/topology?from=100&to=200&live=true"))
      .toBe(topologyCacheKey("/api/v1/projects/p/environments/e/topology?from=200&to=300&live=true"));
  });
  it("isolates historical ranges and environments", () => {
    const first = topologyCacheKey("/api/v1/projects/p/environments/e/topology?from=100&to=200");
    expect(first).not.toBe(topologyCacheKey("/api/v1/projects/p/environments/e/topology?from=200&to=300"));
    expect(first).not.toBe(topologyCacheKey("/api/v1/projects/p/environments/other/topology?from=100&to=200"));
  });
});

it("an older topology cannot overwrite a newer empty historical map", () => {
  let nodes = ["previous-range"];
  expect(applyTopologyResponse(2, 2, () => { nodes = []; })).toBe(true);
  expect(applyTopologyResponse(1, 2, () => { nodes = ["late-old-range"]; })).toBe(false);
  expect(nodes).toEqual([]);
});

it("a superseded initial request cannot publish while its replacement is still loading", () => {
  let applied = false;
  expect(applyTopologyResponse(1, 2, () => { applied = true; })).toBe(false);
  expect(applied).toBe(false);
});
