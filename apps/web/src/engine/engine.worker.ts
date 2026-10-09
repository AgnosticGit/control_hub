/// <reference lib="webworker" />
// Engine Worker: owns the WASM engine. Fetches API responses itself and hands
// the raw JSON text to Rust, so large timelines never become JS object graphs
// on either thread. Seeks and viewport changes are coalesced: only the latest
// state is computed, and each frame carries the generation it reflects.

import init, { WebEngine, engineAbiVersion } from "@karbird/web-engine/pkg";
import {
  ENGINE_ABI_VERSION,
  type Filters,
  type Frame,
  type FrameSummary,
  type InfrastructureSnapshot,
  type LiveEvent,
  type LoadResult,
  type WorkerRequest,
  type WorkerResponse,
} from "@karbird/web-engine";
import { WASM_BYTES, WASM_URL } from "@/generated/wasm-manifest";
import { MAX_RESPONSE_BYTES, countLiveBatchItems, countItems, liveItemEnvelope, liveRetryDelayMs, liveTopologySnapshot, newerSeek, parseLiveAction, parseLiveEdgeChange, parseLiveHead, parseLiveInfrastructure, parseLiveRecord, parseLiveSignals, timelineTail } from "./worker-utils";
import { applyTopologyResponse, topologyCacheKey } from "./topology-cache";

declare const self: DedicatedWorkerGlobalScope;

let engine: WebEngine | null = null;
let pendingSeek: { timeNs: string; generation: number } | null = null;
let pendingViewport: { x: number; y: number; width: number; height: number; scale: number } | null = null;
let pendingFilters: Filters | null = null;
let pendingHopWindow: { beforeNs: string; afterNs: string } | null = null;
let dirty = false;
let scheduled = false;
let publishedTopologyRevision: number | null = null;
let topologyLoadSequence = 0;

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
  if (pendingHopWindow) {
    engine.setHopWindow(BigInt(pendingHopWindow.beforeNs), BigInt(pendingHopWindow.afterNs));
    pendingHopWindow = null;
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
  if (publishedTopologyRevision !== frame.summary.topology_revision) {
    frame.topology = engine.getTopology();
    publishedTopologyRevision = frame.summary.topology_revision;
  }
  post({ kind: "frame", frame }, [
    frame.nodeData.buffer,
    frame.nodeIndices.buffer,
    frame.edgeData.buffer,
    frame.edgeIndices.buffer,
    frame.changedNodes.buffer,
    frame.hopData.buffer,
  ]);
}

const topologyCache = new Map<string, { etag: string; text: string }>();
async function fetchText(url: string, conditional = false): Promise<{ text: string; ms: number }> {
  const t0 = performance.now();
  const key = conditional ? topologyCacheKey(url) : "";
  const cached = conditional ? topologyCache.get(key) : undefined;
  const headers: Record<string,string> = { accept: "application/json" };
  if (cached) headers["if-none-match"] = cached.etag;
  const res = await fetch(url, { credentials: "same-origin", headers });
  if (res.status === 304 && cached) return { text: cached.text, ms: performance.now() - t0 };
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
  const etag = res.headers.get("etag");
  if (conditional && etag) {
    topologyCache.delete(key);
    topologyCache.set(key, { etag, text });
    while (topologyCache.size > 4) {
      const oldest = topologyCache.keys().next().value;
      if (oldest === undefined) break;
      topologyCache.delete(oldest);
    }
  }
  return { text, ms: performance.now() - t0 };
}

// ------------------------------------------------------------------ live channel

interface LiveState {
  url: string;
  ws: WebSocket | null;
  resume: string | null;
  attempt: number;
  timer: ReturnType<typeof setTimeout> | null;
  edgeSubscription: { source: string; target: string; relation: string; afterCursorNs: string } | null;
}

let live: LiveState | null = null;

function postLive(event: LiveEvent) {
  post({ kind: "live", event });
}

function liveConnect() {
  const l = live;
  if (!l) return;
  const u = new URL(l.url, self.location.origin);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  if (l.resume) u.searchParams.set("resume", l.resume);
  postLive({ type: "connecting", attempt: l.attempt });
  const ws = new WebSocket(u);
  l.ws = ws;
  ws.onopen = () => {
    if (live !== l) return;
    l.attempt = 0;
    postLive({ type: "open" });
    sendEdgeSubscription(l);
  };
  ws.onmessage = (ev: MessageEvent<unknown>) => {
    if (live !== l || typeof ev.data !== "string") return;
    onLiveMessage(l, ev.data);
  };
  ws.onclose = (ev: CloseEvent) => {
    if (live !== l) return;
    l.ws = null;
    l.attempt += 1;
    const retryInMs = liveRetryDelayMs(l.attempt);
    postLive({ type: "closed", attempt: l.attempt, retryInMs, code: ev.code });
    l.timer = setTimeout(liveConnect, retryInMs);
  };
}

/** Individual telemetry frames are applied in WASM; only one compact item is parsed on UI. */
function onLiveMessage(l: LiveState, text: string) {
  const head = parseLiveHead(text);
  if (!head) return;
  const { type, resume } = head;
  // Edge replay has its own storage cursor. Never let it advance the telemetry resume token.
  if (resume && type !== "edge_change") l.resume = resume;
  switch (type) {
    case "span":
    case "log":
    case "event": {
      if (!engine) return;
      const signal = parseLiveRecord(text);
      const envelope = liveItemEnvelope(text);
      if (!signal || !envelope) return;
      const t0 = performance.now();
      engine.applyBatch(envelope);
      schedule();
      postLive({
        type: "signal",
        resume: signal.resume,
        signals: [{ kind: signal.kind, resume: signal.resume, storageCursor: signal.storageCursor, record: signal.record }],
        applyMs: performance.now() - t0,
      });
      return;
    }
    case "batch": {
      if (!engine || !resume) return;
      const t0 = performance.now();
      engine.applyBatch(text);
      schedule();
      const signals = parseLiveSignals(text);
      postLive({ type: "batch", items: countLiveBatchItems(text), applyMs: performance.now() - t0, resume, signals });
      return;
    }
    case "infrastructure": {
      if (!engine) return;
      const snapshot = parseLiveInfrastructure(text);
      if (!snapshot) return;
      engine.setInfrastructure(JSON.stringify(snapshot));
      schedule();
      if (resume) postLive({ type: "infrastructure", resume, snapshot });
      return;
    }
    case "action": {
      if (!resume) return;
      const action = parseLiveAction(text);
      if (action) postLive({ type: "action", resume, ...action });
      return;
    }
    case "edge_change": {
      const parsed = parseLiveEdgeChange(text);
      if (!parsed) return;
      const cursor = parsed.change.storage_cursor_ns;
      const subscription = l.edgeSubscription;
      if (subscription && BigInt(cursor) > BigInt(subscription.afterCursorNs)) subscription.afterCursorNs = cursor;
      postLive({ type: "edge_change", resume: parsed.resume ?? cursor, change: parsed.change });
      return;
    }
    case "topology": {
      if (!engine) return;
      const snapshot = liveTopologySnapshot(text);
      if (!snapshot) return;
      try { engine.loadTopology(snapshot); schedule(); }
      catch { /* A later WebSocket topology snapshot can repair a malformed update. */ }
      return;
    }
    case "gap":
      if (resume) postLive({ type: "gap", resume });
      return;
    case "hello":
    case "heartbeat":
      if (resume) postLive({ type: "heartbeat", resume });
      return;
    case "error": {
      let message = "live channel error";
      try {
        const parsed = JSON.parse(text) as { message?: unknown };
        if (typeof parsed.message === "string") message = parsed.message;
      } catch {
        // keep generic message
      }
      postLive({ type: "error", message });
      return;
    }
  }
}

function sendEdgeSubscription(l: LiveState) {
  if (l.ws?.readyState !== WebSocket.OPEN || !l.edgeSubscription) return;
  const { source, target, relation, afterCursorNs } = l.edgeSubscription;
  l.ws.send(JSON.stringify({ type: "subscribe_edge", source, target, relation, after_cursor_ns: afterCursorNs }));
}

function liveStop() {
  const l = live;
  live = null;
  if (!l) return;
  if (l.timer) clearTimeout(l.timer);
  if (l.ws) {
    l.ws.onclose = null;
    l.ws.close(1000, "stopped");
  }
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
      publishedTopologyRevision = null;
      topologyCache.clear();
      return { abiVersion: abi, wasmBytes: WASM_BYTES, initMs: performance.now() - t0 };
    }
    case "loadTopology": {
      const sequence = ++topologyLoadSequence;
      const e = requireEngine();
      const { text, ms } = await fetchText(msg.url, true);
      const t0 = performance.now();
      applyTopologyResponse(sequence, topologyLoadSequence, () => e.loadTopology(text));
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
        logsSource: tail.logs_source,
      };
      return result;
    }
    case "loadInfrastructure": {
      const sequence=++topologyLoadSequence;
      const e = requireEngine();
      const { text } = await fetchText(msg.url);
      applyTopologyResponse(sequence, topologyLoadSequence, () => e.setInfrastructure(text));
      schedule();
      const snapshot=JSON.parse(text) as InfrastructureSnapshot & {topology?:unknown};
      delete snapshot.topology;
      return snapshot;
    }
    case "evictBefore":
      requireEngine().evictBefore(BigInt(msg.beforeNs));
      schedule();
      return null;
    case "seek":
      pendingSeek = newerSeek(pendingSeek, msg);
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
    case "setHopWindow":
      pendingHopWindow = msg;
      schedule();
      return undefined;
    case "nodeMetrics":
      return requireEngine().getNodeMetrics(msg.nodeId, msg.atNs ? BigInt(msg.atNs) : undefined) ?? null;
    case "edgeMetrics":
      return (
        requireEngine().getEdgeMetrics(msg.sourceId, msg.targetId, msg.atNs ? BigInt(msg.atNs) : undefined) ?? null
      );
    case "compare":
      return requireEngine().compare(BigInt(msg.aNs), BigInt(msg.bNs));
    case "nextEvent":
      return requireEngine().nextEventTime(BigInt(msg.afterNs))?.toString() ?? null;
    case "prevEvent":
      return requireEngine().prevEventTime(BigInt(msg.beforeNs))?.toString() ?? null;
    case "pick":
      return requireEngine().pick(msg.x, msg.y) ?? null;
    case "timelineTracks":
      return requireEngine().timelineTracks(BigInt(msg.fromNs), BigInt(msg.toNs), msg.buckets, msg.clipCap);
    case "diagnostics":
      return requireEngine().getDiagnostics();
    case "topology":
      return requireEngine().getTopology();
    case "liveStart":
      requireEngine();
      if (live?.url === msg.url && (live.ws || live.timer)) return null;
      liveStop();
      live = { url: msg.url, ws: null, resume: null, attempt: 0, timer: null, edgeSubscription: null };
      liveConnect();
      return null;
    case "subscribeEdge":
      if (live) {
        live.edgeSubscription = { source: msg.source, target: msg.target, relation: msg.relation, afterCursorNs: msg.afterCursorNs };
        sendEdgeSubscription(live);
      }
      return null;
    case "unsubscribeEdge":
      if (live) {
        live.edgeSubscription = null;
        if (live.ws?.readyState === WebSocket.OPEN) live.ws.send(JSON.stringify({ type: "unsubscribe_edge" }));
      }
      return null;
    case "liveStop":
      liveStop();
      return null;
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
