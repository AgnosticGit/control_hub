// Pure helpers for the link inspector: byte rates, size coverage and the throughput chart.
// Data comes from the engine (`EdgeMetrics`) and `GET …/edges/traffic` (`EdgeTraffic`).

import type { TopologyView, RelationKindDto } from "@karbird/web-engine";
import type { Schemas } from "@/lib/api";

export type EdgeTraffic = Schemas["EdgeTraffic"];

export interface LiveEdgeContribution {
  id: string;
  timeNs: bigint;
  requestBytes: number | null;
  responseBytes: number | null;
  storageCursor: bigint;
}

/** Resolves span-parent pairs and emits one stable contribution for each newly observed map call. */
export class LiveEdgeAccumulator {
  private readonly spans = new Map<string, Record<string, unknown>>();
  private readonly pending = new Map<string, Array<{ span: Record<string, unknown>; storageCursor: string }>>();
  private readonly seen = new Set<string>();
  private readonly seenOrder: string[] = [];

  constructor(
    private readonly edge: SelectedEdge,
    private readonly relation: "calls" | "depends_on",
    private readonly sourceIsService: boolean,
    private readonly baseline: bigint,
  ) {}

  accept(signals: readonly { kind: string; resume: string; storageCursor: string; record: Record<string, unknown> }[]): LiveEdgeContribution[] {
    const out: LiveEdgeContribution[] = [];
    for (const signal of signals) {
      if (signal.kind !== "span") continue;
      const span = signal.record;
      const trace = stringField(span, "trace_id");
      const spanId = stringField(span, "span_id");
      const serviceId = stringField(span, "service_id");
      if (!trace || !spanId || !serviceId) continue;
      const key = `${trace}:${spanId}`;
      const children = this.pending.get(key) ?? [];
      for (const child of children) {
        const contribution = this.callContribution(child.span, span, child.storageCursor);
        if (contribution) out.push(contribution);
      }
      this.pending.delete(key);
      this.spans.set(key, span);
      if (this.spans.size > 5000) this.spans.delete(this.spans.keys().next().value ?? "");

      const kind = numberField(span, "kind");
      const parentId = stringField(span, "parent_span_id");
      const storageCursor = parseBigInt(signal.storageCursor);
      if (storageCursor === null) continue;
      if (this.relation === "depends_on" && kind !== null && [3, 4].includes(kind)
          && serviceId === this.edge.source && stringField(span, "target_entity_id") === this.edge.target) {
        const contribution = this.contribution(span, null, storageCursor);
        if (contribution) out.push(contribution);
      } else if (this.relation === "calls" && !this.sourceIsService && serviceId === this.edge.target
          && kind === 2 && !parentId) {
        const contribution = this.contribution(span, null, storageCursor);
        if (contribution) out.push(contribution);
      } else if (this.relation === "calls" && this.sourceIsService && serviceId === this.edge.target
          && kind !== null && [2, 5].includes(kind) && parentId) {
        const parentKey = `${trace}:${parentId}`;
        const parent = this.spans.get(parentKey);
        if (parent) {
          const contribution = this.callContribution(span, parent, signal.storageCursor);
          if (contribution) out.push(contribution);
        } else {
          const waiting = this.pending.get(parentKey) ?? [];
          waiting.push({ span, storageCursor: signal.storageCursor });
          this.pending.set(parentKey, waiting);
          while ([...this.pending.values()].reduce((count, entries) => count + entries.length, 0) > 5000) {
            const oldestKey = this.pending.keys().next().value;
            if (oldestKey === undefined) break;
            const pending = this.pending.get(oldestKey);
            if (!pending || pending.length <= 1) this.pending.delete(oldestKey);
            else pending.shift();
          }
        }
      }
    }
    return out;
  }

  private callContribution(child: Record<string, unknown>, parent: Record<string, unknown>, storageCursorText: string): LiveEdgeContribution | null {
    if (stringField(parent, "service_id") !== this.edge.source || stringField(child, "service_id") !== this.edge.target) return null;
    const storageCursor = parseBigInt(storageCursorText);
    return storageCursor === null ? null : this.contribution(child, parent, storageCursor);
  }

  private contribution(span: Record<string, unknown>, fallback: Record<string, unknown> | null, storageCursor: bigint): LiveEdgeContribution | null {
    const timeNs = parseBigInt(stringField(span, "start_ns"));
    const trace = stringField(span, "trace_id");
    const spanId = stringField(span, "span_id");
    if (timeNs === null || !trace || !spanId || storageCursor <= this.baseline) return null;
    const id = `${trace}:${spanId}`;
    if (this.seen.has(id)) return null;
    this.seen.add(id);
    this.seenOrder.push(id);
    if (this.seenOrder.length > 5000) {
      const oldest = this.seenOrder.shift();
      if (oldest) this.seen.delete(oldest);
    }
    return {
      id,
      timeNs,
      requestBytes: sizeField(span, "http.request.body.size") ?? (fallback ? sizeField(fallback, "http.request.body.size") : null),
      responseBytes: sizeField(span, "http.response.body.size") ?? (fallback ? sizeField(fallback, "http.response.body.size") : null),
      storageCursor,
    };
  }
}

/** Slide the rolling bucket window and add one deduplicated contribution. */
export function applyLiveEdgeContribution(data: EdgeTraffic, contribution: LiveEdgeContribution, toNs: bigint): EdgeTraffic {
  const bucketNs = BigInt(data.bucket_ns);
  const spanNs = BigInt(data.to_ns) - BigInt(data.from_ns);
  const to = toNs > contribution.timeNs ? toNs : contribution.timeNs + 1n;
  const oldFrom = BigInt(data.from_ns);
  const oldTo = BigInt(data.to_ns);
  const elapsedShift = Number((to - oldTo) / bucketNs);
  const eventShift = contribution.timeNs >= oldTo ? Number((contribution.timeNs - oldTo) / bucketNs) + 1 : 0;
  const shift = Math.max(0, elapsedShift, eventShift);
  const clampShift = Math.max(0, Math.min(data.buckets, shift));
  const from = oldFrom + BigInt(clampShift) * bucketNs;
  const nextTo = from + spanNs;
  const shiftArray = (values: number[]) => [...values.slice(clampShift), ...Array.from({ length: clampShift }, () => 0)];
  const calls = shiftArray(data.calls);
  const sized = shiftArray(data.sized_calls);
  const requestSized = shiftArray(data.request_sized_calls);
  const responseSized = shiftArray(data.response_sized_calls);
  const request = shiftArray(data.request_bytes);
  const response = shiftArray(data.response_bytes);
  const index = Number((contribution.timeNs - from) / bucketNs);
  if (index >= 0 && index < data.buckets) {
    calls[index] = (calls[index] ?? 0) + 1;
    if (contribution.requestBytes !== null || contribution.responseBytes !== null) sized[index] = (sized[index] ?? 0) + 1;
    if (contribution.requestBytes !== null) request[index] = (request[index] ?? 0) + contribution.requestBytes;
    if (contribution.responseBytes !== null) response[index] = (response[index] ?? 0) + contribution.responseBytes;
    if (contribution.requestBytes !== null) requestSized[index] = (requestSized[index] ?? 0) + 1;
    if (contribution.responseBytes !== null) responseSized[index] = (responseSized[index] ?? 0) + 1;
  }
  const total = (values: number[]) => values.reduce((sum, value) => sum + value, 0);
  const totalCalls = total(calls);
  const totalSized = total(sized);
  const totalRequestBytes = total(request);
  const totalResponseBytes = total(response);
  const rangeSeconds = Number(nextTo - from) / 1e9;
  const totalRequestSized = total(requestSized);
  const totalResponseSized = total(responseSized);
  const requestKnown = totalRequestSized > 0;
  const responseKnown = totalResponseSized > 0;
  const peak = (values: number[], known: boolean) => {
    if (!known) return { rate: null, at: null };
    let bestIndex = 0;
    for (let i = 1; i < values.length; i++) if ((values[i] ?? 0) > (values[bestIndex] ?? 0)) bestIndex = i;
    return { rate: ((values[bestIndex] ?? 0) * 1e9) / Number(bucketNs), at: (from + BigInt(bestIndex) * bucketNs).toString() };
  };
  const peakReq = peak(request, requestKnown);
  const peakResp = peak(response, responseKnown);
  return {
    ...data,
    from_ns: from.toString(), to_ns: nextTo.toString(),
    resume_cursor_ns: (contribution.storageCursor > BigInt(data.resume_cursor_ns)
      ? contribution.storageCursor
      : BigInt(data.resume_cursor_ns)).toString(),
    calls, sized_calls: sized, request_sized_calls: requestSized, response_sized_calls: responseSized,
    request_bytes: request, response_bytes: response,
    total_calls: totalCalls, total_sized_calls: totalSized,
    total_request_sized_calls: totalRequestSized,
    total_response_sized_calls: totalResponseSized,
    total_request_bytes: totalRequestBytes, total_response_bytes: totalResponseBytes,
    size_coverage: totalCalls > 0 ? totalSized / totalCalls : null,
    avg_request_bps: requestKnown ? totalRequestBytes / rangeSeconds : null,
    avg_response_bps: responseKnown ? totalResponseBytes / rangeSeconds : null,
    peak_request_bps: peakReq.rate, peak_request_at_ns: peakReq.at,
    peak_response_bps: peakResp.rate, peak_response_at_ns: peakResp.at,
  };
}

function stringField(value: Record<string, unknown>, name: string): string | null {
  return typeof value[name] === "string" && value[name] !== "" ? value[name] as string : null;
}

function numberField(value: Record<string, unknown>, name: string): number | null {
  return typeof value[name] === "number" && Number.isFinite(value[name]) ? value[name] as number : null;
}

function sizeField(value: Record<string, unknown>, name: string): number | null {
  const attrs = value.attributes;
  if (!attrs || typeof attrs !== "object" || Array.isArray(attrs)) return null;
  const raw = (attrs as Record<string, unknown>)[name];
  if (typeof raw !== "string" || !/^\d+$/.test(raw)) return null;
  const size = Number(raw);
  return Number.isSafeInteger(size) && size >= 0 ? size : null;
}

function parseBigInt(value: string | null): bigint | null {
  if (value === null || !/^\d+$/.test(value)) return null;
  try { return BigInt(value); } catch { return null; }
}

/** A map line, by the node IDs at its ends (stable across relayouts, unlike edge indices). */
export interface SelectedEdge {
  source: string;
  target: string;
  relation?: RelationKindDto;
}

/** Node IDs at the ends of engine edge `index`. */
export function edgeEnds(topology: TopologyView | null, index: number): SelectedEdge | null {
  const e = topology?.edges.find((x) => x.index === index);
  const s = e ? topology?.nodes[e.source] : undefined;
  const t = e ? topology?.nodes[e.target] : undefined;
  return s && t && e ? { source: s.id, target: t.id, relation:e.relation } : null;
}

/** Engine edge index of the line `edge.source → edge.target`, or null when it is not on the map. */
export function edgeIndex(topology: TopologyView | null, edge: SelectedEdge | null): number | null {
  if (!topology || !edge) return null;
  const hit = topology.edges.find((e) => topology.nodes[e.source]?.id === edge.source && topology.nodes[e.target]?.id === edge.target && (edge.relation === undefined || edge.relation === e.relation));
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

export function sizeNote(
  s: SizeState,
  directions?: { calls: number; requestSizedCalls: number; responseSizedCalls: number },
): string {
  switch (s.kind) {
    case "no-calls":
      return "No calls over this link in the loaded range.";
    case "unknown":
      return "The services on this link do not report request or response sizes, so data volume is unknown.";
    case "partial":
      if (directions) {
        return `Request size known for ${directions.requestSizedCalls} of ${directions.calls} calls; response size known for ${directions.responseSizedCalls} of ${directions.calls} calls. Rates include only calls with a reported size for that direction.`;
      }
      return `Size known for ${s.sized} of ${s.calls} calls (${formatPercent(s.sized / s.calls)}); rates count only those.`;
    case "full":
      if (directions) {
        return `Request size known for ${directions.requestSizedCalls} of ${directions.calls} calls; response size known for ${directions.responseSizedCalls} of ${directions.calls} calls. Rates include only calls with a reported size for that direction.`;
      }
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
