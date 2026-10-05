// Pure helpers for the link inspector: byte rates, size coverage and the throughput chart.
// Data comes from the engine (`EdgeMetrics`) and `GET …/edges/traffic` (`EdgeTraffic`).

import type { TopologyView } from "@karbird/web-engine";
import type { Schemas } from "@/lib/api";

export type EdgeTraffic = Schemas["EdgeTraffic"];

/** A map line, by the node IDs at its ends (stable across relayouts, unlike edge indices). */
export interface SelectedEdge {
  source: string;
  target: string;
}

/** Node IDs at the ends of engine edge `index`. */
export function edgeEnds(topology: TopologyView | null, index: number): SelectedEdge | null {
  const e = topology?.edges.find((x) => x.index === index);
  const s = e ? topology?.nodes[e.source] : undefined;
  const t = e ? topology?.nodes[e.target] : undefined;
  return s && t ? { source: s.id, target: t.id } : null;
}

/** Engine edge index of the line `edge.source → edge.target`, or null when it is not on the map. */
export function edgeIndex(topology: TopologyView | null, edge: SelectedEdge | null): number | null {
  if (!topology || !edge) return null;
  const hit = topology.edges.find((e) => topology.nodes[e.source]?.id === edge.source && topology.nodes[e.target]?.id === edge.target);
  return hit?.index ?? null;
}

const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** Bytes in binary units with up to three significant digits: `0 B`, `950 B`, `1.2 KB`, `34 MB`. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  let v = bytes;
  let u = 0;
  while (v >= 1024 && u < UNITS.length - 1) {
    v /= 1024;
    u++;
  }
  const digits = u === 0 || v >= 100 ? 0 : v >= 10 ? 1 : 2;
  return `${Number(v.toFixed(digits))} ${UNITS[u]}`;
}

/** Bytes per second; `—` when unknown (no call declared a size). */
export function formatRate(bps: number | null | undefined): string {
  return bps === null || bps === undefined ? "—" : `${formatBytes(bps)}/s`;
}

export function formatMs(ms: number): string {
  if (ms >= 10_000) return `${(ms / 1000).toFixed(1)} s`;
  if (ms >= 100) return `${Math.round(ms)} ms`;
  return `${ms.toFixed(1)} ms`;
}

export function formatPercent(share: number): string {
  const p = share * 100;
  return `${p > 0 && p < 1 ? p.toFixed(1) : Math.round(p)}%`;
}

export type SizeState =
  | { kind: "no-calls" }
  | { kind: "unknown"; calls: number }
  | { kind: "partial"; sized: number; calls: number }
  | { kind: "full"; calls: number };

/** Whether byte rates can be shown, and how much of the traffic they cover. */
export function sizeState(t: Pick<EdgeTraffic, "total_calls" | "total_sized_calls">): SizeState {
  if (t.total_calls <= 0) return { kind: "no-calls" };
  if (t.total_sized_calls <= 0) return { kind: "unknown", calls: t.total_calls };
  if (t.total_sized_calls < t.total_calls) return { kind: "partial", sized: t.total_sized_calls, calls: t.total_calls };
  return { kind: "full", calls: t.total_calls };
}

export function sizeNote(s: SizeState): string {
  switch (s.kind) {
    case "no-calls":
      return "No calls over this link in the loaded range.";
    case "unknown":
      return "The services on this link do not report request or response sizes, so data volume is unknown.";
    case "partial":
      return `Size known for ${s.sized} of ${s.calls} calls (${formatPercent(s.sized / s.calls)}); rates count only those.`;
    case "full":
      return `Size known for all ${s.calls} calls.`;
  }
}

/** Bytes per second of each bucket, request and response combined. */
export function bucketRates(t: Pick<EdgeTraffic, "request_bytes" | "response_bytes" | "bucket_ns">): number[] {
  const seconds = Number(t.bucket_ns) / 1e9;
  if (!(seconds > 0)) return [];
  const n = Math.max(t.request_bytes.length, t.response_bytes.length);
  return Array.from({ length: n }, (_, i) => ((t.request_bytes[i] ?? 0) + (t.response_bytes[i] ?? 0)) / seconds);
}

/** Index of the largest value (the first on ties), or null when all are zero or empty. */
export function peakIndex(values: readonly number[]): number | null {
  let best: number | null = null;
  let bestV = 0;
  values.forEach((v, i) => {
    if (v > bestV) {
      best = i;
      bestV = v;
    }
  });
  return best;
}

/**
 * SVG path of a stepped area chart over `values` in a `width` × `height` box (y down),
 * scaled to the maximum so the peak touches the top.
 */
export function areaPath(values: readonly number[], width: number, height: number): string {
  if (values.length === 0) return "";
  const max = Math.max(...values, 0);
  const step = width / values.length;
  const y = (v: number) => (max > 0 ? height - (v / max) * height : height);
  let d = `M0 ${height}`;
  values.forEach((v, i) => {
    const yy = round(y(v));
    d += ` L${round(i * step)} ${yy} L${round((i + 1) * step)} ${yy}`;
  });
  return `${d} L${round(width)} ${height} Z`;
}

function round(v: number): number {
  return Math.round(v * 10) / 10;
}
