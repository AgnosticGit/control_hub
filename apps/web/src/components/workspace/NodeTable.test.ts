import { expect, it } from "vitest";
import { NODE_STRIDE, type TopologyView } from "@karbird/web-engine";
import { nodeTableRows } from "./NodeTable";

function topology(name: string): TopologyView {
  return { revision: 1, nodes: [{ index: 0, id: name, name, kind: "service", provenance: "observed", x: 300, y: 10000 }], edges: [], infra: null };
}

it("table rows use their frame's graph when the React graph has already advanced", () => {
  const data = new Float32Array(NODE_STRIDE);
  data[5] = 17;
  const rows = nodeTableRows({ topology: topology("frame-service"), nodeIndices: new Uint32Array([0]), nodeData: data }, topology("next-service"));
  expect(rows.map(row => [row.name, row.requests])).toEqual([["frame-service", 17]]);
});

it("table rows retain off-camera cards and use the current graph between revision frames", () => {
  const rows = nodeTableRows({ nodeIndices: new Uint32Array([0]), nodeData: new Float32Array(NODE_STRIDE) }, topology("off-camera-service"));
  expect(rows.map(row => row.name)).toEqual(["off-camera-service"]);
});
