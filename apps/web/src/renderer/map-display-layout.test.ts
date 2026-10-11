import { describe, expect, it } from "vitest";
import type { TopologyView } from "@karbird/web-engine";
import { displayTopologyForMetrics } from "./map-display-layout";

const topology: TopologyView = {
  revision: 7,
  nodes: [
    { index: 0, id: "svc_a", name: "a", kind: "service", provenance: "observed", x: 172, y: 56 },
    { index: 1, id: "svc_b", name: "b", kind: "service", provenance: "observed", x: 344, y: 112 },
  ],
  edges: [{ index: 0, source: 0, target: 1, relation: "calls", provenance: "observed", reasons: [], points: [[258, 56]] }],
  infra: {
    frames: [{ host_id: "host-a", label: "host-a", x: 100, y: 20, width: 400, height: 220, hot: false, has_metrics: true }],
    chips: [{ node: 0, service_id: "svc_a", instance_id: "a-1", instance_ids: ["a-1"], label: "a", host_id: "host-a", x: 172, y: 56 }],
    badges: [{ node: 0, count: 1, hosts: 1 }],
    links: [{ from: [172, 56], to: [344, 112], victim_node: 0, neighbor_node: 1, host_id: "host-a", label: "neighbor" }],
  },
};

describe("displayTopologyForMetrics", () => {
  it("expands node, route, and infrastructure coordinates together while preserving the topology", () => {
    const display = displayTopologyForMetrics(topology, true);

    expect(display.nodes.map(({ x, y }) => [x, y])).toEqual([[220, 104], [440, 208]]);
    expect(display.edges[0]?.points).toEqual([[330, 104]]);
    expect(display.infra?.frames[0]?.x).toBeCloseTo(127.907, 3);
    expect(display.infra?.frames[0]?.y).toBeCloseTo(37.143, 3);
    expect(display.infra?.frames[0]?.width).toBeCloseTo(511.628, 3);
    expect(display.infra?.frames[0]?.height).toBeCloseTo(408.571, 3);
    expect(display.infra?.chips[0]).toMatchObject({ x: 220, y: 104 });
    expect(display.infra?.links[0]).toMatchObject({ from: [220, 104], to: [440, 208] });
    expect(topology.nodes[0]).toMatchObject({ x: 172, y: 56 });
  });

  it("leaves compact map coordinates unchanged", () => {
    expect(displayTopologyForMetrics(topology, false)).toEqual(topology);
  });
});
