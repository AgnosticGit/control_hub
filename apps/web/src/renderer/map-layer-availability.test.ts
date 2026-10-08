import { describe, expect, it } from "vitest";
import { EDGE_STRIDE, FLAG_PROBLEM, NODE_STRIDE } from "@karbird/web-engine";
import { mapLayerAvailability } from "./map-layer-availability";

describe("map layer availability", () => {
  it("disables problem and inferred filters when the current map has neither", () => {
    const frame = {
      nodeIndices: new Uint32Array([1]),
      nodeData: new Float32Array(NODE_STRIDE),
      edgeIndices: new Uint32Array([2]),
      edgeData: new Float32Array(EDGE_STRIDE),
    };
    expect(mapLayerAvailability(frame, [{ provenance: "observed" }])).toEqual({
      ready: true,
      problems: false,
      inferred: false,
    });
  });

  it("enables problem filtering for flagged nodes or edges and inferred filtering for inferred topology", () => {
    const nodeData = new Float32Array(NODE_STRIDE);
    nodeData[6] = FLAG_PROBLEM;
    const frame = {
      nodeIndices: new Uint32Array([1]),
      nodeData,
      edgeIndices: new Uint32Array([2]),
      edgeData: new Float32Array(EDGE_STRIDE),
    };
    expect(mapLayerAvailability(frame, [{ provenance: "inferred" }])).toEqual({
      ready: true,
      problems: true,
      inferred: true,
    });
  });

  it("does not decide availability before the first topology frame", () => {
    expect(mapLayerAvailability(null, null)).toEqual({ ready: false, problems: false, inferred: false });
  });
});
