// Machines on the timeline: CPU and memory per bucket and container events (from
// `GET …/infrastructure/timeline`). Pure helpers; drawing lives in `draw.ts`.

import type { Schemas } from "@/lib/api";
import { xOf, type View } from "./scale";

export type InfraTimeline = Schemas["InfraTimeline"];
export type ContainerEvent = Schemas["ContainerEvent"];
export type ContainerEventKind = Schemas["ContainerEventKind"];

export const EVENT_LABEL: Record<ContainerEventKind, string> = {
  started: "Container started",
  stopped: "Container stopped",
  restarted: "Container restarted",
  redeployed: "Redeployed",
  out_of_memory: "Out of memory",
};

export const EVENT_COLOR: Record<ContainerEventKind, string> = {
  started: "#22c55e",
  stopped: "#9a9fa6",
  restarted: "#f59e0b",
  redeployed: "#3b82f6",
  out_of_memory: "#ef4444",
};

/** The API's longest range and largest bucket count. */
export const MAX_MACHINE_SPAN = 7n * 24n * 3600n * 1_000_000_000n;
export const MAX_MACHINE_BUCKETS = 1000;

/**
 * Range and bucket count to ask the API for so the visible view is covered. Edges snap to a
 * power-of-two grid of about 1/16 of the span, so panning and live following refetch only
 * after the view has moved by a step, not on every frame.
 */
export function machineWindow(view: View, width: number, bucketPx: number): { from: bigint; to: bigint; buckets: number } | null {
  const span = view.to - view.from;
  if (span <= 0n || width <= 0) return null;
  let step = 1_000_000_000n;
  while (step * 16n < span) step *= 2n;
  const floor = (t: bigint) => (t >= 0n ? t / step : (t - step + 1n) / step) * step;
  let from = floor(view.from);
  const to = floor(view.to) + step;
  if (to - from > MAX_MACHINE_SPAN) from = to - MAX_MACHINE_SPAN;
  const wanted = Math.ceil(((width / bucketPx) * Number(to - from)) / Number(span) / 10) * 10;
  return { from, to, buckets: Math.max(1, Math.min(MAX_MACHINE_BUCKETS, wanted)) };
}

/** Agents export every 10–60 s; a value holds over empty buckets for this long (the API's stale limit). */
export const HOLD_NS = 90n * 1_000_000_000n;

function hold(values: Array<number | null>, buckets: number): Array<number | null> {
  let last: number | null = null;
  let age = 0;
  return values.map((v) => {
    if (v !== null) {
      last = v;
      age = 0;
      return v;
    }
    age += 1;
    return age <= buckets ? last : null;
  });
}

/** Fills buckets between two agent exports with the previous value, so narrow buckets draw a continuous track. */
export function holdSamples(t: InfraTimeline): InfraTimeline {
  const bucketNs = BigInt(t.bucket_ns);
  if (bucketNs <= 0n) return t;
  const buckets = Number(HOLD_NS / bucketNs);
  if (buckets < 1) return t;
  return { ...t, hosts: t.hosts.map((h) => ({ ...h, cpu: hold(h.cpu, buckets), memory: hold(h.memory, buckets) })) };
}

export interface MachineGeometry {
  from: bigint;
  bucketNs: number;
  x0: number;
  bucketPx: number;
}

export function machineGeometry(t: InfraTimeline, view: View, width: number): MachineGeometry {
  const from = BigInt(t.from_ns);
  const bucketNs = Math.max(1, Number(t.bucket_ns));
  const pxPerNs = width / Math.max(1, Number(view.to - view.from));
  return { from, bucketNs, x0: xOf(from, view, width), bucketPx: bucketNs * pxPerNs };
}

/** CPU and memory share of machine `host` in the bucket holding `time`; `null` outside or without data. */
export function machineAt(t: InfraTimeline, host: number, time: bigint): { cpu: number | null; memory: number | null } {
  const h = t.hosts[host];
  const from = BigInt(t.from_ns);
  const bucketNs = BigInt(t.bucket_ns);
  if (!h || bucketNs <= 0n || time < from) return { cpu: null, memory: null };
  const i = Number((time - from) / bucketNs);
  if (i >= t.buckets) return { cpu: null, memory: null };
  return { cpu: h.cpu[i] ?? null, memory: h.memory[i] ?? null };
}

export function formatShare(v: number | null): string {
  return v === null ? "–" : `${Math.round(v * 100)}%`;
}

export interface EventMarker {
  event: ContainerEvent;
  time: bigint;
  x: number;
  /** Index into `InfraTimeline.hosts` of the row it sits on. */
  host: number;
}

/** Events inside the visible part of the timeline, placed on their machine's row. */
export function eventMarkers(t: InfraTimeline, view: View, width: number): EventMarker[] {
  const row = new Map(t.hosts.map((h, i) => [h.id, i]));
  const out: EventMarker[] = [];
  for (const event of t.events) {
    const host = row.get(event.host_id);
    if (host === undefined) continue;
    let time: bigint;
    try {
      time = BigInt(event.time_ns);
    } catch {
      continue;
    }
    if (time < view.from || time > view.to) continue;
    out.push({ event, time, host, x: xOf(time, view, width) });
  }
  return out;
}

/** Accessible name of an event marker, e.g. "Container restarted: orders on vps-app". */
export function eventLabel(e: ContainerEvent): string {
  return `${EVENT_LABEL[e.kind]}: ${e.container_name} on ${e.host_name}`;
}
