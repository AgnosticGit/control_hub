/// <reference lib="webworker" />
// Engine Worker: owns the WASM engine. Fetches API responses itself and hands
// the raw JSON text to Rust, so large timelines never become JS object graphs
// on either thread. Seeks and viewport changes are coalesced: only the latest
// state is computed, and each frame carries the generation it reflects.

import init, { WebEngine, engineAbiVersion } from "@controlhub/web-engine/pkg";
import {
  ENGINE_ABI_VERSION,
  type Filters,
  type Frame,
  type FrameSummary,
  type LoadResult,
  type WorkerRequest,
  type WorkerResponse,
} from "@controlhub/web-engine";
import { WASM_BYTES, WASM_URL } from "@/generated/wasm-manifest";

declare const self: DedicatedWorkerGlobalScope;

/** Refuse responses above this size instead of exhausting Worker memory. */
const MAX_RESPONSE_BYTES = 64 * 1024 * 1024;

let engine: WebEngine | null = null;
let pendingSeek: { timeNs: string; generation: number } | null = null;
let pendingViewport: { x: number; y: number; width: number; height: number; scale: number } | null = null;
let pendingFilters: Filters | null = null;
let dirty = false;
let scheduled = false;

function post(msg: WorkerResponse, transfer: Transferable[] = []) {
  self.postMessage(msg, transfer);
}

function requireEngine(): WebEngine {
  if (!engine) throw new Error("engine not initialised");
  return engine;
}

function schedule() {
  dirty = true;
  if (scheduled) return;
  scheduled = true;
  setTimeout(flush, 0);
}

function flush() {
  scheduled = false;
  if (!dirty || !engine) return;
  dirty = false;
  const t0 = performance.now();
  if (pendingFilters) {
    engine.setFilters(pendingFilters);
    pendingFilters = null;
  }
  if (pendingViewport) {
    const v = pendingViewport;
    engine.setViewport(v.x, v.y, v.width, v.height, v.scale);
    pendingViewport = null;
  }
  if (pendingSeek) {
    engine.seek(BigInt(pendingSeek.timeNs), BigInt(pendingSeek.generation));
    pendingSeek = null;
  }
  const vs = engine.getVisibleState() as Omit<Frame, "computeMs" | "summary"> & { summary: FrameSummary };
  const frame: Frame = { ...vs, computeMs: performance.now() - t0 };
  post({ kind: "frame", frame }, [
    frame.nodeData.buffer,
    frame.nodeIndices.buffer,
    frame.edgeData.buffer,
    frame.edgeIndices.buffer,
    frame.changedNodes.buffer,
  ]);
}

async function fetchText(url: string): Promise<{ text: string; ms: number }> {
  const t0 = performance.now();
  const res = await fetch(url, { credentials: "same-origin", headers: { accept: "application/json" } });
  if (!res.ok) {
    let message = `HTTP ${res.status}`;
    try {
      const body = (await res.json()) as { message?: unknown };
      if (typeof body.message === "string") message = `${message}: ${body.message}`;
    } catch {
      // keep status-only message
    }
    throw new Error(message);
  }
  const declared = Number(res.headers.get("content-length") ?? "0");
  if (declared > MAX_RESPONSE_BYTES) throw new Error("response too large for the engine window");
  const text = await res.text();
  if (text.length > MAX_RESPONSE_BYTES) throw new Error("response too large for the engine window");
  return { text, ms: performance.now() - t0 };
}

interface TimelineTail {
  from_ns?: string;
  to_ns?: string;
  spans_truncated?: boolean;
  logs_truncated?: boolean;
  events_truncated?: boolean;
  completeness?: unknown;
}

/**
 * Reads the small metadata tail of a timeline response without parsing the
 * (potentially huge) `items` array in JS. Relies on the server emitting
 * `items` first (TimelineResponse field order); falls back to a full parse.
 */
function timelineTail(text: string): TimelineTail {
  const marker = '],"from_ns":';
  const at = text.lastIndexOf(marker);
  if (at !== -1) {
    try {
      return JSON.parse(`{${text.slice(at + 2)}`) as TimelineTail;
    } catch {
      // fall through
    }
  }
  const full = JSON.parse(text) as TimelineTail;
  return full;
}

function countItems(text: string): number {
  let n = 0;
  let i = text.indexOf('"type":"');
  while (i !== -1) {
    n++;
    i = text.indexOf('"type":"', i + 8);
  }
  return n;
}

async function handle(msg: WorkerRequest): Promise<unknown> {
  switch (msg.kind) {
    case "init": {
      const t0 = performance.now();
      await init({ module_or_path: WASM_URL });
      const abi = engineAbiVersion();
      if (abi !== ENGINE_ABI_VERSION) {
        throw new Error(`engine ABI mismatch: wasm=${abi} ui=${ENGINE_ABI_VERSION}; rebuild with \`just wasm\``);
      }
      engine?.free();
      engine = new WebEngine(msg.config ? JSON.stringify(msg.config) : "");
      return { abiVersion: abi, wasmBytes: WASM_BYTES, initMs: performance.now() - t0 };
    }
    case "loadTopology": {
      const e = requireEngine();
      const { text, ms } = await fetchText(msg.url);
      const t0 = performance.now();
      e.loadTopology(text);
      const parsed = JSON.parse(text) as { nodes: unknown[]; completeness?: unknown; from_ns?: string; to_ns?: string };
      schedule();
      const result: LoadResult = {
        count: parsed.nodes.length,
        fetchMs: ms,
        applyMs: performance.now() - t0,
        truncated: false,
        completeness: parsed.completeness ?? null,
        fromNs: parsed.from_ns,
        toNs: parsed.to_ns,
      };
      return result;
    }
    case "loadTimeline": {
      const e = requireEngine();
      const { text, ms } = await fetchText(msg.url);
      const t0 = performance.now();
      e.applyBatch(text);
      const applyMs = performance.now() - t0;
      const tail = timelineTail(text);
      schedule();
      const result: LoadResult = {
        count: countItems(text),
        fetchMs: ms,
        applyMs,
        truncated: Boolean(tail.spans_truncated || tail.logs_truncated || tail.events_truncated),
        completeness: tail.completeness ?? null,
        fromNs: tail.from_ns,
        toNs: tail.to_ns,
      };
      return result;
    }
    case "evictBefore":
      requireEngine().evictBefore(BigInt(msg.beforeNs));
      schedule();
      return null;
    case "seek":
      if (!pendingSeek || msg.generation >= pendingSeek.generation) pendingSeek = msg;
      schedule();
      return undefined;
    case "setViewport":
      pendingViewport = msg;
      schedule();
      return undefined;
    case "setFilters":
      pendingFilters = msg.filters;
      schedule();
      return undefined;
    case "nodeMetrics":
      return requireEngine().getNodeMetrics(msg.nodeId, msg.atNs ? BigInt(msg.atNs) : undefined) ?? null;
    case "compare":
      return requireEngine().compare(BigInt(msg.aNs), BigInt(msg.bNs));
    case "pick":
      return requireEngine().pick(msg.x, msg.y) ?? null;
    case "diagnostics":
      return requireEngine().getDiagnostics();
    case "topology":
      return requireEngine().getTopology();
  }
}

self.onmessage = (ev: MessageEvent<WorkerRequest>) => {
  const msg = ev.data;
  const id = "id" in msg ? msg.id : null;
  handle(msg).then(
    (value) => {
      if (id !== null) post({ kind: "result", id, value });
    },
    (err: unknown) => {
      const message = err instanceof Error ? err.message : String(err);
      if (id !== null) post({ kind: "error", id, message });
      else post({ kind: "fatal", message });
    },
  );
};
