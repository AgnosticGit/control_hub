import { EDGE_STRIDE, FLAG_PROBLEM, NODE_STRIDE } from "@karbird/web-engine";
import type { Frame, TopologyViewEdge } from "@karbird/web-engine";

export interface MapLayerAvailability {
  ready: boolean;
  problems: boolean;
  inferred: boolean;
}

/** Reads layer availability from the same current frame and backend topology used by the map. */
export function mapLayerAvailability(
  frame: Pick<Frame, "nodeData" | "nodeIndices" | "edgeData" | "edgeIndices"> | null,
  topologyEdges: readonly Pick<TopologyViewEdge, "provenance">[] | null,
): MapLayerAvailability {
  if (!frame || !topologyEdges) return { ready: false, problems: false, inferred: false };

  let problems = false;
  for (let i = 0; i < frame.nodeIndices.length && !problems; i++) {
    problems = ((frame.nodeData[i * NODE_STRIDE + 6] ?? 0) & FLAG_PROBLEM) !== 0;
  }
  for (let i = 0; i < frame.edgeIndices.length && !problems; i++) {
    problems = ((frame.edgeData[i * EDGE_STRIDE + 7] ?? 0) & FLAG_PROBLEM) !== 0;
  }

  return {
    ready: true,
    problems,
    inferred: topologyEdges.some((edge) => edge.provenance === "inferred"),
  };
}
