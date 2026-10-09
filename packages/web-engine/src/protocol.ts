// Messages between the UI thread and the engine Worker.
//
// Time values cross the boundary as decimal nanosecond strings (never JS
// numbers, which lose precision past 2^53). Requests carrying an `id` get
// exactly one `result` or `error` reply; frames are pushed.

import type {
  Diagnostics,
  EdgeMetrics,
  EngineConfig,
  Filters,
  FrameSummary,
  InfrastructureSnapshot,
  NodeDiff,
  NodeMetrics,
  TimelineTracks,
  TopologyView,
} from "./generated";

export type WorkerRequest =
  | { kind: "init"; id: number; config?: Partial<EngineConfig> }
  | { kind: "loadTopology"; id: number; url: string }
  | { kind: "loadTimeline"; id: number; url: string }
  /** Machines and instances for the infrastructure layer (`InfrastructureSnapshot`). */
  | { kind: "loadInfrastructure"; id: number; url: string }
  | { kind: "evictBefore"; id: number; beforeNs: string }
  | { kind: "seek"; timeNs: string; generation: number }
  | { kind: "setViewport"; x: number; y: number; width: number; height: number; scale: number }
  | { kind: "setFilters"; filters: Filters }
  /** Frames carry request hops overlapping `[time - beforeNs, time + afterNs]`. */
  | { kind: "setHopWindow"; beforeNs: string; afterNs: string }
  | { kind: "nodeMetrics"; id: number; nodeId: string; atNs?: string }
  /** Traffic over the edge `sourceId → targetId` in the rolling window (`EdgeMetrics`). */
  | { kind: "edgeMetrics"; id: number; sourceId: string; targetId: string; atNs?: string }
  | { kind: "compare"; id: number; aNs: string; bNs: string }
  /** Time of the first recorded item strictly after `afterNs`, or null. */
  | { kind: "nextEvent"; id: number; afterNs: string }
  /** Time of the last recorded item strictly before `beforeNs`, or null. */
  | { kind: "prevEvent"; id: number; beforeNs: string }
  | { kind: "pick"; id: number; x: number; y: number }
  /** Layered timeline aggregates over `[fromNs, toNs)`; see `TimelineTracks`. */
  | { kind: "timelineTracks"; id: number; fromNs: string; toNs: string; buckets: number; clipCap: number }
  | { kind: "diagnostics"; id: number }
  | { kind: "topology"; id: number }
  /** Open (or re-target) the live WebSocket; `url` is a same-origin path. */
  | { kind: "liveStart"; id: number; url: string }
  | { kind: "liveStop"; id: number };

/** Live channel status pushed by the Worker. */
export type LiveEvent =
  | { type: "connecting"; attempt: number }
  | { type: "open" }
  | { type: "closed"; attempt: number; retryInMs: number; code: number }
  | { type: "signal"; resume: string; signals: [LiveSignal]; applyMs: number }
  /** Legacy frame accepted only during rolling upgrades; the server no longer emits it. */
  | { type: "batch"; items: number; applyMs: number; resume: string; signals: LiveSignal[] }
  | { type: "infrastructure"; resume: string; snapshot: InfrastructureSnapshot }
  | { type: "action"; resume: string; plan: Record<string, unknown>; snapshot: boolean }
  | { type: "heartbeat"; resume: string }
  /** The server requires a resume reconnect to continue recovery. */
  | { type: "gap"; resume: string }
  | { type: "error"; message: string };

/** Full, already-redacted records delivered to side panels over the same live channel. */
export type LiveSignal =
  | { kind: "span"; resume: string; storageCursor: string; record: Record<string, unknown> }
  | { kind: "log"; resume: string; storageCursor: string; record: Record<string, unknown> }
  | { kind: "event"; resume: string; storageCursor: string; record: Record<string, unknown> };

export interface InitResult {
  abiVersion: number;
  wasmBytes: number;
  initMs: number;
}

export interface LoadResult {
  /** Items (or nodes for topology) received from the API. */
  count: number;
  fetchMs: number;
  applyMs: number;
  /** Server-side truncation / completeness flags passed through verbatim. */
  truncated: boolean;
  completeness: unknown;
  fromNs?: string;
  toNs?: string;
  /** Timeline only: where the log items came from (`hot`, `archive`, `mixed`). */
  logsSource?: string;
}

export interface ResultMap {
  init: InitResult;
  loadTopology: LoadResult;
  loadTimeline: LoadResult;
  loadInfrastructure: InfrastructureSnapshot;
  evictBefore: null;
  nodeMetrics: NodeMetrics | null;
  edgeMetrics: EdgeMetrics | null;
  compare: NodeDiff[];
  nextEvent: string | null;
  prevEvent: string | null;
  pick: string | null;
  timelineTracks: TimelineTracks;
  diagnostics: Diagnostics;
  topology: TopologyView;
  liveStart: null;
  liveStop: null;
}

export type RpcKind = keyof ResultMap;

export interface Frame {
  /** Metadata paired with this frame when the topology revision changes. */
  topology?: TopologyView;
  nodeData: Float32Array;
  nodeIndices: Uint32Array;
  edgeData: Float32Array;
  edgeIndices: Uint32Array;
  changedNodes: Uint32Array;
  /** Request hops, `HOP_STRIDE` floats each, times relative to `summary.time_ns` in ms. */
  hopData: Float32Array;
  summary: FrameSummary;
  /** Time spent computing the frame inside the Worker (WASM + copy). */
  computeMs: number;
}

export type WorkerResponse =
  | { kind: "result"; id: number; value: unknown }
  | { kind: "error"; id: number; message: string }
  | { kind: "frame"; frame: Frame }
  | { kind: "live"; event: LiveEvent }
  | { kind: "fatal"; message: string };
