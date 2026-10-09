// What every mark on the system map means. Colours come from the renderer, so the key shows
// exactly what the map draws.

import { ACCENT, EDGE_ERROR, EDGE_INFERRED, EDGE_OK, EDGE_WARN, EXTERNAL_COLOR, STATUS_COLORS, hex } from "@/renderer/map-colors";
import { colorAt, cssColor } from "@/renderer/service-colors";

/** Sample service colour for marks drawn in "the service's colour". */
export const SAMPLE_SERVICE = cssColor(colorAt(0));

export type Swatch =
  | { kind: "dot"; color: string; size?: "small" | "normal" | "large"; halo?: string; faded?: boolean }
  | { kind: "ring"; color: string }
  | { kind: "line"; color: string; dashed?: boolean; thick?: boolean }
  | { kind: "card"; color: string; dashed?: boolean; glow?: string; faded?: boolean; outline?: string; corner?: string };

export interface KeyItem {
  id: string;
  swatch: Swatch;
  label: string;
  meaning: string;
}

export interface KeySection {
  title: string;
  items: KeyItem[];
}

const [NO_DATA, HEALTHY, DEGRADED, FAILING] = STATUS_COLORS.map(hex);

export const MAP_KEY: KeySection[] = [
  {
    title: "Moving dots: recorded requests",
    items: [
      {
        id: "request",
        swatch: { kind: "dot", color: SAMPLE_SERVICE },
        label: "Filled dot",
        meaning: "A request on its way to the service it calls. Its colour is the service where the request started.",
      },
      {
        id: "wait",
        swatch: { kind: "dot", color: SAMPLE_SERVICE, size: "small", faded: true },
        label: "Small pale dot",
        meaning: "The request waits at the called service while that service works.",
      },
      {
        id: "response",
        swatch: { kind: "ring", color: SAMPLE_SERVICE },
        label: "Ring",
        meaning: "The response on its way back to the caller.",
      },
      {
        id: "failed",
        swatch: { kind: "dot", color: SAMPLE_SERVICE, halo: hex(EDGE_ERROR) },
        label: "Red halo",
        meaning: "The call failed. Its response ring is red too.",
      },
      {
        id: "selected-request",
        swatch: { kind: "dot", color: SAMPLE_SERVICE, size: "large" },
        label: "Large dot",
        meaning: "Part of the request you selected.",
      },
      {
        id: "dimmed-request",
        swatch: { kind: "dot", color: SAMPLE_SERVICE, faded: true },
        label: "Faded dot",
        meaning: "A service you switched off, or not part of the selected request.",
      },
    ],
  },
  {
    title: "Lines: calls between components",
    items: [
      {
        id: "observed",
        swatch: { kind: "line", color: hex(EDGE_OK) },
        label: "Solid line",
        meaning: "Observed: these calls are in the recorded spans. Thicker means more calls; the arrow points to the callee.",
      },
      {
        id: "inferred",
        swatch: { kind: "line", color: hex(EDGE_INFERRED), dashed: true },
        label: "Dashed line",
        meaning: "Inferred: derived from span attributes (a database, a URL), not seen as a call between two services.",
      },
      {
        id: "external-line",
        swatch: { kind: "line", color: hex(EXTERNAL_COLOR) },
        label: "Light line",
        meaning: "A call to or from something outside the project.",
      },
      {
        id: "warn-line",
        swatch: { kind: "line", color: hex(EDGE_WARN) },
        label: "Amber line",
        meaning: "5% or more of the calls on this link fail.",
      },
      {
        id: "error-line",
        swatch: { kind: "line", color: hex(EDGE_ERROR) },
        label: "Red line",
        meaning: "Half or more of the calls on this link fail.",
      },
      {
        id: "selected-line",
        swatch: { kind: "line", color: hex(ACCENT), thick: true },
        label: "Bright cyan line",
        meaning: "A link the selected request went through.",
      },
    ],
  },
  {
    title: "Cards: services and dependencies",
    items: [
      {
        id: "service",
        swatch: { kind: "card", color: SAMPLE_SERVICE, corner: HEALTHY ?? hex(ACCENT) },
        label: "Service card",
        meaning: "Solid rounded card. Its border and icon use the service colour; the small corner dot shows data health.",
      },
      {
        id: "healthy",
        swatch: { kind: "dot", color: HEALTHY ?? hex(ACCENT), size: "small" },
        label: "Cyan corner dot",
        meaning: "Healthy.",
      },
      {
        id: "degraded",
        swatch: { kind: "card", color: DEGRADED ?? hex(EDGE_WARN), glow: DEGRADED ?? hex(EDGE_WARN) },
        label: "Amber card",
        meaning: "Degraded: 5% or more of its requests fail.",
      },
      {
        id: "failing",
        swatch: { kind: "card", color: FAILING ?? hex(EDGE_ERROR), glow: FAILING ?? hex(EDGE_ERROR) },
        label: "Red card",
        meaning: "Failing: half or more of its requests fail.",
      },
      {
        id: "no-data",
        swatch: { kind: "dot", color: NO_DATA ?? "#64748b", size: "small", faded: true },
        label: "Grey corner dot",
        meaning: "No data in the current window.",
      },
      {
        id: "external-card",
        swatch: { kind: "card", color: hex(EXTERNAL_COLOR), dashed: true },
        label: "Dashed card",
        meaning: "Outside the project: an external client or dependency.",
      },
      {
        id: "inferred-card",
        swatch: { kind: "card", color: SAMPLE_SERVICE, faded: true },
        label: "Pale frame",
        meaning: "Known only from what other services report about it.",
      },
      {
        id: "selected-card",
        swatch: { kind: "card", color: SAMPLE_SERVICE, outline: "#ffffff" },
        label: "White outline",
        meaning: "The component open in the side panel.",
      },
    ],
  },
];
