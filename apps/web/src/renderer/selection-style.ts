export interface MapSelection {
  node: number | null;
  edge: number | null;
  edgeNodes: readonly [number, number] | null;
  host: string | null;
}

/** Keep the selection legible while gently quieting the rest of the map. */
export const SELECTION_DIM_ALPHA = 0.48;

/** Width of the continuous focus ribbon beneath an incident edge. */
export function incidentGlowWidth(edgeWidth: number, inferred: boolean): number {
  return edgeWidth + (inferred ? 6 : 4);
}

/** Clicking the selected map item again clears it; any different item replaces the selection. */
export function toggleSelection<T>(selected: T | null, clicked: T | null): T | null {
  return clicked !== null && clicked === selected ? null : clicked;
}

export function selectionAlpha(selection: MapSelection): number {
  return selection.node !== null || selection.edge !== null || selection.host !== null ? SELECTION_DIM_ALPHA : 1;
}

export function nodeFocused(node: number, hostId: string | null, selection: MapSelection): boolean {
  if (selection.node === null && selection.edge === null && selection.host === null) return true;
  return selection.node === node || selection.edgeNodes?.includes(node) === true || (hostId !== null && selection.host === hostId);
}

export function chipFocused(chip: { node: number; hostId: string }, selection: MapSelection): boolean {
  if (selection.node === null && selection.edge === null && selection.host === null) return true;
  return selection.node === chip.node || selection.edgeNodes?.includes(chip.node) === true || selection.host === chip.hostId;
}

/** Edge indices connected to a selected logical node, regardless of edge direction. */
export function incidentEdgeIndices(
  node: number | null,
  edges: readonly { index: number; source: number; target: number }[],
): Set<number> {
  if (node === null) return new Set();
  return new Set(edges.filter((edge) => edge.source === node || edge.target === node).map((edge) => edge.index));
}

export function frameFocused(hostId: string, selection: MapSelection): boolean {
  if (selection.node === null && selection.edge === null && selection.host === null) return true;
  return selection.host === hostId;
}
