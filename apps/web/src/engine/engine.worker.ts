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
import { MAX_RESPONSE_BYTES, countLiveBatchItems, countItems, liveItemEnvelope, liveRetryDelayMs, liveTopologySnapshot, newerSeek, parseLiveAction, parseLiveEdgeChange, parseLiveHead, parseLiveInfrastructure, parseLiveRecord, parseLiveSignals, parseTimelineFrame, timelineChunkEnvelope, timelineChunkItemsJson } from "./worker-utils";
import { applyTopologyResponse, topologyCacheKey } from "./topology-cache";
import { missingTimelineRanges, openTimelineCache, type TimelineRange } from "./timeline-cache";

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
  scopeKey: string;
  ws: WebSocket | null;
  resume: string | null;
  attempt: number;
  timer: ReturnType<typeof setTimeout> | null;
  edgeSubscription: { source: string; target: string; relation: string; afterCursorNs: string } | null;
  openWaiters: Set<{ resolve: () => void; reject: (error: Error) => void }>;
  temporary: boolean;
}

interface PendingHistory {
  requestId: string;
  scopeKey: string;
  fromNs: string;
  toNs: string;
  liveUrl: string;
  nextLiveUrl?: string;
  ranges: TimelineRange[];
  covered: TimelineRange[];
  uncacheableRanges: Set<string>;
  resolve: (result: LoadResult) => void;
  reject: (error: Error) => void;
  count: number;
  networkCount: number;
  applyMs: number;
  startedAt: number;
  sentSocket: WebSocket | null;
}

let live: LiveState | null = null;
let pendingHistory: PendingHistory | null = null;
let liveCacheSequence = 0;
let timelineLoadQueue: Promise<void> = Promise.resolve();

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
    for (const waiter of l.openWaiters) waiter.resolve();
    l.openWaiters.clear();
    sendEdgeSubscription(l);
    void sendPendingTimelineLoad(l);
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
  const history = parseTimelineFrame(text);
  if (history) {
    void onTimelineHistoryFrame(l, text, history).catch((error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      failPendingHistory(l, history.requestId, message);
    });
    return;
  }
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
      cacheLiveItem(l.scopeKey, signal.item);
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

function cacheLiveItem(scopeKey: string, item: Record<string, unknown>) {
  if (!scopeKey) return;
  const timestamp = [item.start_ns, item.time_ns, item.end_ns].find((value) => typeof value === "string" && /^\d+$/.test(value));
  if (typeof timestamp !== "string") return;
  const from = BigInt(timestamp);
  const to = from < 9_223_372_036_854_775_806n ? from + 1n : from;
  if (to <= from) return;
  const itemsJson = JSON.stringify([item]);
  const sequence = liveCacheSequence++;
  void openTimelineCache().then((cache) => cache.putChunk(scopeKey, from.toString(), to.toString(), sequence, itemsJson)).catch(() => {});
}

function sendEdgeSubscription(l: LiveState) {
  if (l.ws?.readyState !== WebSocket.OPEN || !l.edgeSubscription) return;
  const { source, target, relation, afterCursorNs } = l.edgeSubscription;
  l.ws.send(JSON.stringify({ type: "subscribe_edge", source, target, relation, after_cursor_ns: afterCursorNs }));
}

function liveStop() {
  if (pendingHistory) {
    if (live) live.temporary = true;
    return;
  }
  const l = live;
  live = null;
  if (!l) return;
  if (l.timer) clearTimeout(l.timer);
  for (const waiter of l.openWaiters) waiter.reject(new Error("live connection stopped"));
  l.openWaiters.clear();
  if (l.ws) {
    l.ws.onclose = null;
    l.ws.close(1000, "stopped");
  }
}

function ensureLiveOpen(url: string): Promise<LiveState> {
  if (live && pendingHistory && live.url !== url) {
    url = live.url;
    pendingHistory.liveUrl = url;
  }
  if (!live || live.url !== url) {
    liveStop();
    live = { url, scopeKey: pendingHistory?.scopeKey ?? "", ws: null, resume: null, attempt: 0, timer: null, edgeSubscription: null, openWaiters: new Set(), temporary: true };
    liveCacheSequence = Date.now() * 1000;
    liveConnect();
  }
  const current = live;
  if (!current) return Promise.reject(new Error("live connection unavailable"));
  if (current.ws?.readyState === WebSocket.OPEN) return Promise.resolve(current);
  return new Promise((resolve, reject) => current.openWaiters.add({ resolve: () => resolve(current), reject }));
}

function finishHistoryConnection(l: LiveState, pending: PendingHistory): void {
  if (live !== l) return;
  if (pending.nextLiveUrl) {
    const url = pending.nextLiveUrl;
    liveStop();
    live = { url, scopeKey: pending.scopeKey, ws: null, resume: null, attempt: 0, timer: null, edgeSubscription: null, openWaiters: new Set(), temporary: false };
    liveConnect();
  } else if (l.temporary) {
    liveStop();
  }
}

function failPendingHistory(l: LiveState | null, requestId: string, message: string): void {
  const pending = pendingHistory;
  if (!pending || pending.requestId !== requestId) return;
  if (l?.ws?.readyState === WebSocket.OPEN) l.ws.send(JSON.stringify({ type: "timeline_cancel", request_id: requestId }));
  pendingHistory = null;
  pending.reject(new Error(message));
  if (l) finishHistoryConnection(l, pending);
}

async function sendPendingTimelineLoad(l: LiveState): Promise<void> {
  const pending = pendingHistory;
  if (!pending || pending.liveUrl !== l.url || l.ws?.readyState !== WebSocket.OPEN) return;
  if (pending.sentSocket === l.ws) return;
  const ranges = pending.ranges.flatMap((range) => missingTimelineRanges(
    BigInt(range.fromNs), BigInt(range.toNs), pending.covered,
  ));
  if (ranges.length === 0) {
    pending.resolve({ count: pending.count, fetchMs: performance.now() - pending.startedAt, applyMs: pending.applyMs, truncated: false, completeness: null, fromNs: pending.fromNs, toNs: pending.toNs, newItems: pending.networkCount });
    pendingHistory = null;
    finishHistoryConnection(l, pending);
    return;
  }
  l.ws.send(JSON.stringify({
    type: "timeline_load", request_id: pending.requestId, from_ns: pending.fromNs, to_ns: pending.toNs,
    limit: 50_000, ranges: ranges.slice(0, 64).map((range) => ({ from_ns: range.fromNs, to_ns: range.toNs })),
  }));
  pending.sentSocket = l.ws;
}

async function onTimelineHistoryFrame(l: LiveState, text: string, frame: NonNullable<ReturnType<typeof parseTimelineFrame>>): Promise<void> {
  const pending = pendingHistory;
  if (!pending || pending.requestId !== frame.requestId || live !== l || !engine) return;
  if (frame.type === "chunk") {
    const envelope = timelineChunkEnvelope(text);
    if (!envelope) return;
    const started = performance.now();
    engine.applyBatch(envelope);
    schedule();
    pending.applyMs += performance.now() - started;
    const itemCount = countItems(envelope);
    pending.count += itemCount;
    pending.networkCount += itemCount;
    if (pending.scopeKey) {
      try {
        const cache = await openTimelineCache();
        const itemsJson = timelineChunkItemsJson(text);
        const stored = itemsJson && await cache.putChunk(pending.scopeKey, frame.fromNs, frame.toNs, frame.sequence, itemsJson);
        if (!stored) pending.uncacheableRanges.add(`${frame.fromNs}:${frame.toNs}`);
      } catch { pending.uncacheableRanges.add(`${frame.fromNs}:${frame.toNs}`); }
    }
    if (pendingHistory === pending && l.ws?.readyState === WebSocket.OPEN) {
      l.ws.send(JSON.stringify({ type: "timeline_ack", request_id: frame.requestId, sequence: frame.sequence }));
    }
    return;
  }
  if (frame.type === "range_complete") {
    if (pending.scopeKey && !pending.uncacheableRanges.has(`${frame.fromNs}:${frame.toNs}`)) {
      try {
        const cache = await openTimelineCache();
        await cache.markComplete(pending.scopeKey, frame.fromNs, frame.toNs);
      } catch { /* Continue without durable caching. */ }
    }
    if (pendingHistory !== pending) return;
    pending.covered.push({ fromNs: frame.fromNs, toNs: frame.toNs });
    if (l.ws?.readyState === WebSocket.OPEN) {
      l.ws.send(JSON.stringify({ type: "timeline_ack", request_id: frame.requestId, sequence: frame.sequence }));
    }
    return;
  }
  if (frame.type === "error") {
    pendingHistory = null;
    pending.reject(new Error(frame.message));
    finishHistoryConnection(l, pending);
    return;
  }
  if (!frame.truncated) {
    const remaining = pending.ranges.flatMap((range) => missingTimelineRanges(
      BigInt(range.fromNs), BigInt(range.toNs), pending.covered,
    ));
    if (remaining.length > 0) {
      pending.sentSocket = null;
      await sendPendingTimelineLoad(l);
      return;
    }
  }
  pendingHistory = null;
  const result: LoadResult = {
    count: pending.count,
    fetchMs: performance.now() - pending.startedAt,
    applyMs: pending.applyMs,
    truncated: frame.truncated,
    completeness: frame.completeness,
    fromNs: pending.fromNs,
    toNs: pending.toNs,
    logsSource: frame.logsSource,
    newItems: pending.networkCount,
  };
  pending.resolve(result);
  finishHistoryConnection(l, pending);
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
      const startedAt = performance.now();
      const completed: TimelineRange[] = [];
      let cachedCount = 0;
      let cachedApplyMs = 0;
      if (msg.scopeKey) {
        try {
          const cache = await openTimelineCache();
          const cached = await cache.read(msg.scopeKey, msg.fromNs, msg.toNs);
          completed.push(...cached.covered);
          const t0 = performance.now();
          for (const chunk of cached.chunks) {
            e.applyBatch(`{"items":${chunk.itemsJson}}`);
            cachedCount += countItems(chunk.itemsJson);
          }
          schedule();
          if (cached.chunks.length) cachedApplyMs = performance.now() - t0;
        } catch { /* Offline and private browsing remain usable without IndexedDB. */ }
      }
      const ranges = missingTimelineRanges(BigInt(msg.fromNs), BigInt(msg.toNs), completed);
      if (ranges.length === 0) {
        return { count: cachedCount, fetchMs: performance.now() - startedAt, applyMs: cachedApplyMs, truncated: false, completeness: null, fromNs: msg.fromNs, toNs: msg.toNs, newItems: 0 } satisfies LoadResult;
      }
      if (pendingHistory) throw new Error("another timeline load is already active");
      return new Promise<LoadResult>((resolve, reject) => {
        const requestId = crypto.randomUUID();
        pendingHistory = {
          requestId, scopeKey: msg.scopeKey, fromNs: msg.fromNs, toNs: msg.toNs, liveUrl: msg.liveUrl,
          ranges, covered: completed, uncacheableRanges: new Set(), resolve, reject, count: cachedCount, networkCount: 0, applyMs: cachedApplyMs, startedAt,
          sentSocket: null,
        };
        void ensureLiveOpen(msg.liveUrl).then(async (connection) => {
          try { await sendPendingTimelineLoad(connection); }
          catch (error) { failPendingHistory(connection, requestId, error instanceof Error ? error.message : String(error)); }
        }, (error: unknown) => failPendingHistory(live, requestId, error instanceof Error ? error.message : String(error)));
      });
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
      if (pendingHistory && live && live.url !== msg.url) {
        pendingHistory.nextLiveUrl = msg.url;
        return null;
      }
      if (live?.url === msg.url && (live.ws || live.timer)) {
        live.scopeKey = msg.scopeKey ?? live.scopeKey;
        live.temporary = false;
        return null;
      }
      liveStop();
      live = { url: msg.url, scopeKey: msg.scopeKey ?? "", ws: null, resume: null, attempt: 0, timer: null, edgeSubscription: null, openWaiters: new Set(), temporary: false };
      liveCacheSequence = Date.now() * 1000;
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
  if (msg.kind !== "init" && !engine) {
    requestsBeforeInit.push({ msg, id });
    return;
  }
  dispatch(msg, id);
};

const requestsBeforeInit: Array<{ msg: WorkerRequest; id: number | null }> = [];

function dispatch(msg: WorkerRequest, id: number | null) {
  const operation = msg.kind === "loadTimeline"
    ? timelineLoadQueue.then(() => handle(msg))
    : handle(msg);
  if (msg.kind === "loadTimeline") timelineLoadQueue = operation.then(() => undefined, () => undefined);
  operation.then(
    (value) => {
      if (id !== null) post({ kind: "result", id, value });
      if (msg.kind === "init") drainRequestsBeforeInit();
    },
    (err: unknown) => {
      const message = err instanceof Error ? err.message : String(err);
      if (id !== null) post({ kind: "error", id, message });
      else post({ kind: "fatal", message });
      if (msg.kind === "init") drainRequestsBeforeInit(message);
    },
  );
}

function drainRequestsBeforeInit(initError?: string) {
  const queued = requestsBeforeInit.splice(0);
  for (const request of queued) {
    if (initError) {
      if (request.id !== null) post({ kind: "error", id: request.id, message: initError });
      continue;
    }
    dispatch(request.msg, request.id);
  }
}
