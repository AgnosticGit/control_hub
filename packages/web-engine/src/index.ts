export type * from "./generated";
export * from "./protocol";

/** Must equal `engineAbiVersion()` exported by the WASM build. */
export const ENGINE_ABI_VERSION = 3;

/** Per-node stride in `nodeData`: x, y, radius, status, active, window_requests, flags. */
export const NODE_STRIDE = 7;
/** Per-edge stride in `edgeData`: x1, y1, x2, y2, active, window_calls, error_ratio, flags. */
export const EDGE_STRIDE = 8;

/**
 * Per-hop stride in `hopData`: edge_index, t0, t1, t2, t3, flags, origin. Times are milliseconds
 * relative to `summary.time_ns`: request leaves the caller (t0), reaches the callee (t1), the
 * callee answers (t2), the answer is back (t3). `origin` is the node index of the service where
 * the hop's trace started, or -1 when unknown.
 */
export const HOP_STRIDE = 7;
export const HOP_ERROR = 1;
/** Callee not instrumented: only t0 and t3 are known (t1 = t0, t2 = t3). */
export const HOP_LEAF = 2;
export const HOP_HIGHLIGHT = 4;
/** Clock skew between services; the callee's timestamps were shifted or clamped into order. */
export const HOP_CLAMPED = 8;
/** On an edge touching a switched-off (muted) service. */
export const HOP_DIMMED = 16;

export const FLAG_HIGHLIGHT = 1;
export const FLAG_INFERRED = 2;
export const FLAG_DECLARED = 4;
export const FLAG_RESOURCE = 8;
export const FLAG_PROBLEM = 16;
/** Node or edge belongs to a switched-off (muted) service: drawn dimmed. */
export const FLAG_DIMMED = 32;
/** Node outside the project's architecture (external dependency or client), or an edge touching one. */
export const FLAG_EXTERNAL = 64;

/** Health status encoding used in render buffers. */
export const STATUS = { unknown: 0, healthy: 1, degraded: 2, failing: 3 } as const;
