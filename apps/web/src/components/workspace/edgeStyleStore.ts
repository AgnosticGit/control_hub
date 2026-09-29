import { usePersisted, writePersisted } from "@/lib/persisted";
import { DEFAULT_EDGE_STYLE, isEdgeStyle, type EdgeStyle } from "@/renderer/edge-path";

const KEY = "controlhub.map.edgeStyle";

export function setEdgeStyle(style: EdgeStyle) {
  writePersisted(KEY, style);
}

/** The map line style chosen by the user, remembered per browser. */
export function useEdgeStyle(): EdgeStyle {
  return usePersisted(KEY, DEFAULT_EDGE_STYLE, (raw) => (isEdgeStyle(raw) ? raw : null));
}
