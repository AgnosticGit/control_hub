import type { InfraView, TopologyView } from "@karbird/web-engine";
import { COMPACT_CARD_SIZE, EXPANDED_CARD_SIZE } from "./map-card-geometry";

export interface MapDisplayScale {
  x: number;
  y: number;
}

export function mapDisplayScale(serviceMetrics: boolean): MapDisplayScale {
  return serviceMetrics
    ? { x: EXPANDED_CARD_SIZE.width / COMPACT_CARD_SIZE.width, y: EXPANDED_CARD_SIZE.height / COMPACT_CARD_SIZE.height }
    : { x: 1, y: 1 };
}

export function displayMapPoint(x: number, y: number, scale: MapDisplayScale): [number, number] {
  return [x * scale.x, y * scale.y];
}

export function sourceMapPoint(x: number, y: number, scale: MapDisplayScale): [number, number] {
  return [x / scale.x, y / scale.y];
}

function scaleInfra(infra: InfraView, scale: MapDisplayScale): InfraView {
  return {
    ...infra,
    frames: infra.frames.map((frame) => ({
      ...frame,
      x: frame.x * scale.x,
      y: frame.y * scale.y,
      width: frame.width * scale.x,
      height: frame.height * scale.y,
    })),
    chips: infra.chips.map((chip) => ({ ...chip, x: chip.x * scale.x, y: chip.y * scale.y })),
    links: infra.links.map((link) => ({
      ...link,
      from: displayMapPoint(link.from[0], link.from[1], scale),
      to: displayMapPoint(link.to[0], link.to[1], scale),
    })),
  };
}

/** Reflow every topology decoration in the renderer's display space; the server layout stays canonical. */
export function displayTopologyForMetrics(topology: TopologyView, serviceMetrics: boolean): TopologyView {
  if (!serviceMetrics) return topology;
  const scale = mapDisplayScale(true);
  return {
    ...topology,
    nodes: topology.nodes.map((node) => ({ ...node, x: node.x * scale.x, y: node.y * scale.y })),
    edges: topology.edges.map((edge) => ({
      ...edge,
      points: edge.points.map(([x, y]) => displayMapPoint(x, y, scale)),
    })),
    infra: topology.infra ? scaleInfra(topology.infra, scale) : null,
  };
}
