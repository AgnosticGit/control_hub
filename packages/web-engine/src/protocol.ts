// Messages between the UI thread and the engine Worker.
//
// Time values cross the boundary as decimal nanosecond strings (never JS
// numbers, which lose precision past 2^53). Requests carrying an `id` get
// exactly one `result` or `error` reply; frames are pushed.

import type {
  Diagnostics,
  EngineConfig,
  Filters,
  FrameSummary,
  NodeDiff,
  NodeMetrics,
  TimelineTracks,
  TopologyView,
} from "./generated";

export type WorkerRequest =
  | { kind: "init"; id: number; config?: Partial<EngineConfig> }
  | { kind: "loadTopology"; id: number; url: string }
  | { kind: "loadTimeline"; id: number; url: string }
  | { kind: "evictBefore"; id: number; beforeNs: string }
  | { kind: "seek"; timeNs: string; generation: number }
  | { kind: "setViewport"; x: number; y: number; width: number; height: number; scale: number }
  | { kind: "setFilters"; filters: Filters }
  /** Frames carry request hops overlapping `[time - beforeNs, time + afterNs]`. */
  | { kind: "setHopWindow"; beforeNs: string; afterNs: string }
  | { kind: "nodeMetrics"; id: number; nodeId: string; atNs?: string }
  | { kind: "compare"; id: number; aNs: string; bNs: string }
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
  | { type: "batch"; items: number; applyMs: number; resume: string }
  | { type: "heartbeat"; resume: string }
  /** The server skipped rows (size limit); reload the range over HTTP. */
  | { type: "gap"; resume: string }
  | { type: "error"; message: string };

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
}

export interface ResultMap {
  init: InitResult;
  loadTopology: LoadResult;
  loadTimeline: LoadResult;
  evictBefore: null;
  nodeMetrics: NodeMetrics | null;
  compare: NodeDiff[];
  pick: string | null;
  timelineTracks: TimelineTracks;
  diagnostics: Diagnostics;
  topology: TopologyView;
  liveStart: null;
  liveStop: null;
}

export type RpcKind = keyof ResultMap;

export interface Frame {
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
