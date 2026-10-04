import { usePersisted, writePersisted } from "@/lib/persisted";
import { DEFAULT_EDGE_STYLE, isEdgeStyle, type EdgeStyle } from "@/renderer/edge-path";

export const EDGE_STYLE_KEY = "karbird.map.edgeStyle";

export function parseEdgeStyle(raw: string): EdgeStyle | null {
  return isEdgeStyle(raw) ? raw : null;
}

export function setEdgeStyle(style: EdgeStyle) {
  writePersisted(EDGE_STYLE_KEY, style);
}

/** The map line style chosen by the user, remembered per browser. */
export function useEdgeStyle(): EdgeStyle {
  return usePersisted(EDGE_STYLE_KEY, DEFAULT_EDGE_STYLE, parseEdgeStyle);
}
