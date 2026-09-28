export type * from "./generated";
export * from "./protocol";

/** Must equal `engineAbiVersion()` exported by the WASM build. */
export const ENGINE_ABI_VERSION = 1;

/** Per-node stride in `nodeData`: x, y, radius, status, active, window_requests, flags. */
export const NODE_STRIDE = 7;
/** Per-edge stride in `edgeData`: x1, y1, x2, y2, active, window_calls, error_ratio, flags. */
export const EDGE_STRIDE = 8;

export const FLAG_HIGHLIGHT = 1;
export const FLAG_INFERRED = 2;
export const FLAG_DECLARED = 4;
export const FLAG_RESOURCE = 8;
export const FLAG_PROBLEM = 16;

/** Health status encoding used in render buffers. */
export const STATUS = { unknown: 0, healthy: 1, degraded: 2, failing: 3 } as const;
