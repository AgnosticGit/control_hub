// Synthetic demo system for the public landing page, drawn the way the workspace draws a real one.
// Pure and deterministic: the picture at a given time depends only on that time, which is the
// point the demo makes about replay.

import { colorAt } from "@/renderer/service-colors";

export type Health = "ok" | "degraded" | "failing";
export type Orientation = "wide" | "narrow";

export interface MapNode {
  id: string;
  label: string;
  external?: boolean;
  /** Known only from an inferred edge, no spans of its own yet. */
  inferred?: boolean;
  /** Centre of the card in the wide (left to right) and narrow (top to bottom) layouts. */
  wide: readonly [number, number];
  narrow: readonly [number, number];
}

export interface MapEdge {
  from: string;
  to: string;
  /** Observed from spans, or inferred (with the reason shown on hover). */
  kind: "observed" | "inferred";
  reason?: string;
}

export interface Marker {
  t: number;
  kind: "deploy" | "action";
  label: string;
}

export interface RequestDot {
  edge: number;
  /** 0..1 along the edge. */
  at: number;
  error: boolean;
}

export interface DemoState {
  t: number;
  health: Record<string, Health>;
  p95ms: Record<string, number>;
  errorRate: Record<string, number>;
  requests: Record<string, number>;
  dots: RequestDot[];
}

export interface Bucket {
  t: number;
  requests: number;
  errors: number;
  latency: number;
  events: number;
}

/** Length of the demo window in seconds; `WINDOW` is "now". */
export const WINDOW = 900;
export const FAULT_START = 540;
export const FAULT_FAILING = 600;
export const SPREAD_START = 660;
export const BUCKET = 10;

export const VIEW: Record<Orientation, { w: number; h: number; card: { w: number; h: number } }> = {
  wide: { w: 920, h: 360, card: { w: 150, h: 50 } },
  narrow: { w: 360, h: 520, card: { w: 108, h: 46 } },
};

export const NODES: readonly MapNode[] = [
  { id: "clients", label: "External clients", external: true, wide: [90, 190], narrow: [180, 40] },
  { id: "gateway", label: "gateway", wide: [275, 190], narrow: [180, 140] },
  { id: "auth", label: "auth", wide: [460, 75], narrow: [62, 250] },
  { id: "catalog", label: "catalog", wide: [460, 175], narrow: [180, 250] },
  { id: "orders", label: "orders", wide: [460, 280], narrow: [298, 250] },
  { id: "inventory", label: "inventory", wide: [645, 130], narrow: [62, 365] },
  { id: "payments", label: "payments", wide: [645, 225], narrow: [180, 365] },
  { id: "notifications", label: "notifications", inferred: true, wide: [645, 315], narrow: [298, 365] },
  { id: "provider", label: "payment API", external: true, wide: [830, 225], narrow: [180, 475] },
];

export const EDGES: readonly MapEdge[] = [
  { from: "clients", to: "gateway", kind: "observed" },
  { from: "gateway", to: "auth", kind: "observed" },
  { from: "gateway", to: "catalog", kind: "observed" },
  { from: "gateway", to: "orders", kind: "observed" },
  { from: "orders", to: "inventory", kind: "observed" },
  { from: "orders", to: "payments", kind: "observed" },
  { from: "orders", to: "notifications", kind: "inferred", reason: "Inferred from messaging attributes: no spans from notifications yet." },
  { from: "payments", to: "provider", kind: "observed" },
];

export const MARKERS: readonly Marker[] = [
  { t: 528, kind: "deploy", label: "orders 2.14 deployed" },
  { t: 760, kind: "action", label: "Plan: clear faults on orders — awaiting 2nd approval" },
];

/** Requests per edge: departure period and travel time in seconds. */
const SCHEDULE = EDGES.map((_, i) => ({ period: 1.4 + (i % 3) * 0.5, travel: 1.2 + (i % 2) * 0.4, offset: (i * 0.37) % 1 }));

/** Requests per 30 s each node handles while healthy. */
const BASE_REQUESTS: Record<string, number> = {
  clients: 0,
  gateway: 760,
  auth: 240,
  catalog: 410,
  orders: 150,
  inventory: 145,
  payments: 120,
  notifications: 0,
  provider: 118,
};

export function clampT(t: number): number {
  if (!Number.isFinite(t)) return WINDOW;
  return Math.min(WINDOW, Math.max(0, t));
}

function hash(s: string): number {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 997;
  return h;
}

/** Deterministic noise in [-1, 1] for a key and an integer step. */
function noise(key: number, step: number): number {
  const x = Math.sin(key * 12.9898 + step * 78.233) * 43758.5453;
  return (x - Math.floor(x)) * 2 - 1;
}

export function healthAt(id: string, t: number): Health {
  const time = clampT(t);
  if (id === "orders") {
    if (time >= FAULT_FAILING) return "failing";
    if (time >= FAULT_START) return "degraded";
  }
  if ((id === "gateway" || id === "payments") && time >= SPREAD_START) return "degraded";
  return "ok";
}

function latencyAt(id: string, t: number): number {
  const health = healthAt(id, t);
  const base = 18 + (hash(id) % 40);
  if (health === "failing") return base + 160;
  if (health === "degraded") return base + (id === "orders" ? 95 : 40);
  return base;
}

function errorRateAt(id: string, t: number): number {
  const health = healthAt(id, t);
  if (health === "failing") return 0.6;
  if (health === "degraded") return id === "orders" ? 0.12 : 0.04;
  return 0;
}

/** Requests in the last 30 s, as the card sub-label shows them. */
export function requestsAt(id: string, t: number): number {
  const time = clampT(t);
  const base = id === "clients" ? (BASE_REQUESTS.gateway ?? 0) : (BASE_REQUESTS[id] ?? 0);
  if (base === 0) return 0;
  const drop = healthAt(id === "clients" ? "gateway" : id, time) === "failing" ? 0.7 : 1;
  return Math.round(base * drop * (1 + 0.06 * noise(hash(id), Math.floor(time / BUCKET))));
}

/** Request dots on the edges at time `t`. */
export function dotsAt(t: number): RequestDot[] {
  const out: RequestDot[] = [];
  EDGES.forEach((edge, i) => {
    const s = SCHEDULE[i];
    if (!s || edge.kind === "inferred") return;
    const failing = healthAt(edge.to, t) === "failing" || healthAt(edge.from, t) === "failing";
    for (let k = 0; k < 3; k++) {
      const depart = Math.floor(t / s.period - s.offset) - k;
      const start = (depart + s.offset) * s.period;
      const at = (t - start) / s.travel;
      if (at < 0 || at > 1) continue;
      out.push({ edge: i, at, error: failing && depart % 5 < 3 });
    }
  });
  return out;
}

export function stateAt(t: number, dotClock = t): DemoState {
  const time = clampT(t);
  const health: Record<string, Health> = {};
  const p95ms: Record<string, number> = {};
  const errorRate: Record<string, number> = {};
  const requests: Record<string, number> = {};
  for (const n of NODES) {
    health[n.id] = healthAt(n.id, time);
    p95ms[n.id] = latencyAt(n.id, time);
    errorRate[n.id] = errorRateAt(n.id, time);
    requests[n.id] = requestsAt(n.id, time);
  }
  return { t: time, health, p95ms, errorRate, requests, dots: dotsAt(dotClock) };
}

/** Timeline tracks for the whole window, one bucket per `BUCKET` seconds. */
export function timelineBuckets(): Bucket[] {
  const out: Bucket[] = [];
  for (let i = 0; i < WINDOW / BUCKET; i++) {
    const t = i * BUCKET;
    const mid = t + BUCKET / 2;
    const rate = errorRateAt("orders", mid);
    const requests = Math.round(250 * (1 + 0.12 * noise(7, i)) * (healthAt("orders", mid) === "failing" ? 0.82 : 1));
    out.push({
      t,
      requests,
      errors: Math.round(requests * rate * 0.35 * (1 + 0.2 * noise(11, i))),
      latency: latencyAt("orders", mid) * (1 + 0.08 * noise(13, i)),
      events: MARKERS.some((m) => m.t >= t && m.t < t + BUCKET) ? 3 : noise(17, i) > 0.55 ? 1 : 0,
    });
  }
  return out;
}

/** "now" or "−m:ss" relative to the end of the window. */
export function relativeLabel(t: number): string {
  const back = Math.round(WINDOW - clampT(t));
  if (back === 0) return "now";
  const m = Math.floor(back / 60);
  const s = back % 60;
  return `−${m}:${s.toString().padStart(2, "0")}`;
}

/** Service colours the way the workspace assigns them: services ordered by id, externals excluded. */
export function serviceColor(id: string): number | null {
  const services = NODES.filter((n) => !n.external)
    .map((n) => n.id)
    .sort();
  const i = services.indexOf(id);
  return i < 0 ? null : colorAt(i);
}

export type Curve = readonly [readonly [number, number], readonly [number, number], readonly [number, number], readonly [number, number]];

/** Cubic curve of an edge, leaving the source card and entering the target card along the flow. */
export function edgeCurve(edge: MapEdge, o: Orientation): Curve | null {
  const a = NODES.find((n) => n.id === edge.from);
  const b = NODES.find((n) => n.id === edge.to);
  if (!a || !b) return null;
  const { card } = VIEW[o];
  const [ax, ay] = a[o];
  const [bx, by] = b[o];
  if (o === "wide") {
    const sx = ax + card.w / 2;
    const tx = bx - card.w / 2 - 6;
    const d = (tx - sx) / 2;
    return [
      [sx, ay],
      [sx + d, ay],
      [tx - d, by],
      [tx, by],
    ];
  }
  const sy = ay + card.h / 2;
  const ty = by - card.h / 2 - 6;
  const d = (ty - sy) / 2;
  return [
    [ax, sy],
    [ax, sy + d],
    [bx, ty - d],
    [bx, ty],
  ];
}

export function curvePath(c: Curve): string {
  const [p, c1, c2, q] = c;
  return `M${p[0]} ${p[1]} C${c1[0]} ${c1[1]} ${c2[0]} ${c2[1]} ${q[0]} ${q[1]}`;
}

/** Point at parameter `f` (clamped to 0..1) on a cubic curve. */
export function pointOnCurve(c: Curve, f: number): [number, number] {
  const t = Math.min(1, Math.max(0, f));
  const u = 1 - t;
  const [p, c1, c2, q] = c;
  const w = [u * u * u, 3 * u * u * t, 3 * u * t * t, t * t * t] as const;
  return [w[0] * p[0] + w[1] * c1[0] + w[2] * c2[0] + w[3] * q[0], w[0] * p[1] + w[1] * c1[1] + w[2] * c2[1] + w[3] * q[1]];
}
