"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Filters, Frame, FrameSummary, InitResult, LoadResult, TopologyView } from "@controlhub/web-engine";
import { EngineClient } from "@/engine/client";
import { scopedUrl } from "@/lib/api";
import { NS_PER_MS, NS_PER_SEC, clampNs, nowNs, parseNs } from "@/lib/time";
import type { AnimClock } from "@/renderer/hops";
import { playStart, replayStep } from "./timeline/range";

export type Mode = "live" | "paused" | "replay" | "historical";
/** Replay speeds; 0.001x shows one millisecond of recorded time per second. */
export const SPEEDS = [0.001, 0.01, 0.1, 0.5, 1, 2, 10] as const;
export type Speed = (typeof SPEEDS)[number];

/**
 * Live request animation runs this far behind the wall clock: SDKs export spans in batches
 * (OpenTelemetry's default is every 5 s), so the newest requests are not loaded yet at "now".
 */
export const LIVE_ANIM_DELAY_MS = 8000;

export function animClockFor(mode: Mode, speed: Speed): AnimClock {
  if (mode === "live") return { kind: "wall", delayMs: LIVE_ANIM_DELAY_MS };
  return { kind: "frame", rate: mode === "replay" ? speed : 0 };
}

/** Hop window around the frame time that covers the animation time until the next frame. */
export function hopWindowFor(mode: Mode): { before: bigint; after: bigint } {
  if (mode === "live") {
    // Frames come at least every LIVE_TICK_MS; animation time is frame time - delay + elapsed.
    return { before: BigInt(LIVE_ANIM_DELAY_MS + 1000) * NS_PER_MS, after: 500n * NS_PER_MS };
  }
  if (mode === "replay") return { before: 500n * NS_PER_MS, after: 1000n * NS_PER_MS };
  return { before: 0n, after: 0n };
}

const LIVE_WINDOW_NS = 15n * 60n * NS_PER_SEC;
/** Live clock: advances the cursor (and polls when the WebSocket is unavailable). */
const LIVE_TICK_MS = 2000;
const LIVE_FALLBACK_AFTER = 3;
const EVICT_EVERY_MS = 10_000;
const TOPOLOGY_POLL_MS = 10_000;
const OVERLAP_NS = 10n * NS_PER_SEC;
const TIMELINE_LIMIT = "50000";
const EVICT_ALL = 9223372036854775807n;

export interface Completeness {
  watermark_ns: string | null;
  processing_caught_up: boolean;
  unresolved_processing_errors: number;
  truncated: boolean;
  sampling: string;
  notes: string[];
}

export interface LoadInfo {
  items: number;
  fetchMs: number;
  applyMs: number;
  truncated: boolean;
  completeness: Completeness | null;
  at: number;
}

export interface EngineState {
  phase: "starting" | "ready" | "error";
  error: string | null;
  init: InitResult | null;
  mode: Mode;
  speed: Speed;
  rangeFrom: bigint;
  rangeTo: bigint;
  cursor: bigint;
  /** Replay range picked on the timeline: Play loops inside it until it is cleared. */
  loop: { from: bigint; to: bigint } | null;
  summary: FrameSummary | null;
  frameMs: number;
  topology: TopologyView | null;
  load: LoadInfo | null;
  loading: boolean;
  liveTransport: "off" | "connecting" | "websocket" | "polling";
}

export function asCompleteness(v: unknown): Completeness | null {
  return v && typeof v === "object" && "processing_caught_up" in v ? (v as Completeness) : null;
}

export function useWorkspaceEngine(projectId: string, environmentId: string) {
  const clientRef = useRef<EngineClient | null>(null);
  const frameListeners = useRef(new Set<(f: Frame) => void>());
  const lastFrameRef = useRef<Frame | null>(null);
  const [state, setState] = useState<EngineState>(() => {
    const now = nowNs();
    return {
      phase: "starting",
      error: null,
      init: null,
      mode: "live",
      speed: 1,
      rangeFrom: now - LIVE_WINDOW_NS,
      rangeTo: now,
      cursor: now,
      loop: null,
      summary: null,
      frameMs: 0,
      topology: null,
      load: null,
      loading: false,
      liveTransport: "off",
    };
  });
  // Mirrors `state` synchronously so callbacks (rAF loop, pollers) never read a
  // value older than the last patch.
  const stateRef = useRef(state);

  const patch = useCallback((p: Partial<EngineState>) => {
    stateRef.current = { ...stateRef.current, ...p };
    setState((s) => ({ ...s, ...p }));
  }, []);

  const url = useCallback(
    (resource: string, from: bigint, to: bigint, extra: Record<string, string> = {}) =>
      scopedUrl(projectId, environmentId, resource, { from: from.toString(), to: to.toString(), ...extra }),
    [projectId, environmentId],
  );

  const refreshTopologyView = useCallback(async () => {
    const c = clientRef.current;
    if (!c) return;
    patch({ topology: await c.call({ kind: "topology" }) });
  }, [patch]);

  const recordLoad = useCallback(
    (r: LoadResult) => {
      patch({
        load: {
          items: r.count,
          fetchMs: r.fetchMs,
          applyMs: r.applyMs,
          truncated: r.truncated,
          completeness: asCompleteness(r.completeness),
          at: Date.now(),
        },
      });
    },
    [patch],
  );

  const loadRange = useCallback(
    async (from: bigint, to: bigint, opts: { reset: boolean; topology: boolean }) => {
      const c = clientRef.current;
      if (!c) return;
      patch({ loading: true });
      try {
        if (opts.reset) await c.call({ kind: "evictBefore", beforeNs: EVICT_ALL.toString() });
        if (opts.topology) {
          await c.call({ kind: "loadTopology", url: url("topology", from, to) });
        }
        const r = await c.call({ kind: "loadTimeline", url: url("timeline", from, to, { limit: TIMELINE_LIMIT }) });
        recordLoad(r);
        if (opts.topology) await refreshTopologyView();
        patch({ error: null });
      } catch (e) {
        patch({ error: e instanceof Error ? e.message : String(e) });
      } finally {
        patch({ loading: false });
      }
    },
    [patch, url, recordLoad, refreshTopologyView],
  );

  const seek = useCallback(
    (t: bigint) => {
      const s = stateRef.current;
      const cursor = clampNs(t, s.rangeFrom, s.rangeTo);
      clientRef.current?.seek(cursor);
      patch({ cursor });
    },
    [patch],
  );

  // Engine lifecycle.
  useEffect(() => {
    const client = new EngineClient();
    clientRef.current = client;
    // React state gets frame summaries at most every SUMMARY_MS; the trailing
    // frame is always delivered so the UI never shows a stale summary.
    const SUMMARY_MS = 120;
    let lastSummaryAt = 0;
    let trailing: ReturnType<typeof setTimeout> | null = null;
    let lastRevision: number | null = null;
    const publish = () => {
      trailing = null;
      lastSummaryAt = performance.now();
      const f = lastFrameRef.current;
      if (f) patch({ summary: f.summary, frameMs: f.computeMs });
    };
    const offFrame = client.onFrame((f) => {
      lastFrameRef.current = f;
      for (const l of frameListeners.current) l(f);
      if (lastRevision !== null && lastRevision !== f.summary.topology_revision) {
        void client.call({ kind: "topology" }).then((topology) => patch({ topology }));
      }
      lastRevision = f.summary.topology_revision;
      const wait = SUMMARY_MS - (performance.now() - lastSummaryAt);
      if (wait <= 0) publish();
      else if (!trailing) trailing = setTimeout(publish, wait);
    });
    const offFatal = client.onFatal((message) => patch({ phase: "error", error: message }));
    (async () => {
      try {
        const init = await client.call({ kind: "init" });
        patch({ phase: "ready", init });
        const now = nowNs();
        const from = now - LIVE_WINDOW_NS;
        patch({ rangeFrom: from, rangeTo: now, cursor: now });
        await loadRange(from, now, { reset: false, topology: true });
        // Recompute at the current cursor: the user may have paused or seeked during the load.
        client.seek(stateRef.current.cursor);
      } catch (e) {
        patch({ phase: "error", error: e instanceof Error ? e.message : String(e) });
      }
    })();
    return () => {
      if (trailing) clearTimeout(trailing);
      offFrame();
      offFatal();
      client.dispose();
      clientRef.current = null;
    };
  }, [loadRange, patch]);

  // Live: the Worker holds a WebSocket that pushes new items straight into the engine.
  // After repeated connection failures we fall back to HTTP polling with overlap (the
  // engine deduplicates by id) until the socket reconnects.
  useEffect(() => {
    if (state.phase !== "ready" || state.mode !== "live") return;
    const c = clientRef.current;
    if (!c) return;
    let cancelled = false;
    let busy = false;
    let lastTopology = Date.now();
    let lastEvict = Date.now();

    const tick = async () => {
      if (cancelled || busy) return;
      busy = true;
      try {
        const now = nowNs();
        if (stateRef.current.liveTransport === "polling") {
          const from = stateRef.current.rangeTo - OVERLAP_NS;
          recordLoad(await c.call({ kind: "loadTimeline", url: url("timeline", from, now, { limit: TIMELINE_LIMIT }) }));
        }
        if (Date.now() - lastEvict > EVICT_EVERY_MS) {
          lastEvict = Date.now();
          await c.call({ kind: "evictBefore", beforeNs: (now - LIVE_WINDOW_NS).toString() });
        }
        if (Date.now() - lastTopology > TOPOLOGY_POLL_MS) {
          lastTopology = Date.now();
          const r = await c.call({ kind: "loadTopology", url: url("topology", now - LIVE_WINDOW_NS, now) });
          await refreshTopologyView();
          const prev = stateRef.current.load;
          if (prev) patch({ load: { ...prev, completeness: asCompleteness(r.completeness) ?? prev.completeness } });
        }
        // The user may have paused or seeked while this tick was awaiting; never move their cursor.
        if (cancelled || stateRef.current.mode !== "live") return;
        c.seek(now);
        patch({ rangeFrom: now - LIVE_WINDOW_NS, rangeTo: now, cursor: now, error: null });
      } catch (e) {
        patch({ error: e instanceof Error ? e.message : String(e) });
      } finally {
        busy = false;
      }
    };

    const offLive = c.onLive((ev) => {
      switch (ev.type) {
        case "open":
          patch({ liveTransport: "websocket" });
          return;
        case "closed":
          if (ev.attempt >= LIVE_FALLBACK_AFTER) patch({ liveTransport: "polling" });
          return;
        case "batch": {
          const prev = stateRef.current.load;
          patch({
            liveTransport: "websocket",
            load: {
              items: ev.items,
              fetchMs: 0,
              applyMs: ev.applyMs,
              truncated: false,
              completeness: prev?.completeness ?? null,
              at: Date.now(),
            },
          });
          return;
        }
        case "gap": {
          // The server skipped rows over its size limit: reload the live window over HTTP.
          const now = nowNs();
          void c
            .call({ kind: "loadTimeline", url: url("timeline", now - LIVE_WINDOW_NS, now, { limit: TIMELINE_LIMIT }) })
            .then(recordLoad, (e: unknown) => patch({ error: e instanceof Error ? e.message : String(e) }));
          return;
        }
        case "error":
          patch({ error: `live: ${ev.message}` });
          return;
        default:
          return;
      }
    });
    patch({ liveTransport: "connecting" });
    void c.call({ kind: "liveStart", url: scopedUrl(projectId, environmentId, "live", {}) });
    const id = setInterval(() => void tick(), LIVE_TICK_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
      offLive();
      void c.call({ kind: "liveStop" });
      patch({ liveTransport: "off" });
    };
  }, [state.phase, state.mode, url, recordLoad, refreshTopologyView, patch, projectId, environmentId]);

  useEffect(() => {
    if (state.phase !== "ready") return;
    const w = hopWindowFor(state.mode);
    clientRef.current?.setHopWindow(w.before, w.after);
  }, [state.phase, state.mode]);

  // Replay: advance the cursor at `speed` x real time; stop at the loaded end or loop the range.
  useEffect(() => {
    if (state.mode !== "replay") return;
    let raf = 0;
    let last = performance.now();
    const step = (t: number) => {
      const dtMs = t - last;
      last = t;
      const s = stateRef.current;
      const advance = BigInt(Math.round(dtMs * s.speed * 1e6));
      const next = replayStep(s.cursor, advance, s.rangeTo, s.loop);
      clientRef.current?.seek(next.cursor);
      if (next.stop) {
        patch({ cursor: next.cursor, mode: "paused" });
        return;
      }
      patch({ cursor: next.cursor });
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [state.mode, patch]);

  const actions = useMemo(() => {
    const a = {
    pause: () => patch({ mode: stateRef.current.mode === "historical" ? "historical" : "paused" }),
    play: () => {
      const s = stateRef.current;
      const start = playStart(s.cursor, s.rangeFrom, s.rangeTo, s.loop);
      if (start !== s.cursor) seek(start);
      patch({ mode: "replay" });
    },
    /** Sets or clears the replay range; picking one leaves live mode. */
    setLoop: (loop: { from: bigint; to: bigint } | null) => {
      const s = stateRef.current;
      patch({ loop, mode: loop && s.mode === "live" ? "paused" : s.mode });
      if (loop && (s.cursor < loop.from || s.cursor > loop.to)) seek(loop.from);
    },
    setSpeed: (speed: Speed) => patch({ speed }),
    /**
     * Loops `loop` at `speed` from its start: replays one request from beginning to end. The
     * range is (re)loaded first: replay stops the live stream, and a request picked seconds
     * after it happened may not have reached the engine completely yet.
     */
    playRange: (loop: { from: bigint; to: bigint }, speed: Speed) => {
      patch({ loop, speed, mode: "replay" });
      seek(loop.from);
      void loadRange(loop.from, loop.to, { reset: false, topology: false });
    },
    seek: (t: bigint) => {
      const s = stateRef.current;
      if (s.mode === "live" || s.mode === "replay") patch({ mode: "paused" });
      seek(t);
    },
    stepPrev: async () => {
      const f = await clientRef.current?.settledFrame();
      const p = parseNs(f?.summary.prev_event_ns);
      if (p !== null) a.seek(p);
    },
    stepNext: async () => {
      const f = await clientRef.current?.settledFrame();
      const n = parseNs(f?.summary.next_event_ns);
      if (n !== null) a.seek(n);
    },
    jump: (deltaMs: number) => a.seek(stateRef.current.cursor + BigInt(deltaMs) * NS_PER_MS),
    returnToLive: async () => {
      const s = stateRef.current;
      const now = nowNs();
      const wasHistorical = s.mode === "historical" || now - s.rangeTo > LIVE_WINDOW_NS;
      patch({ mode: "live", rangeFrom: now - LIVE_WINDOW_NS, rangeTo: now, cursor: now, loop: null });
      await loadRange(wasHistorical ? now - LIVE_WINDOW_NS : s.rangeTo - OVERLAP_NS, now, {
        reset: wasHistorical,
        topology: true,
      });
      clientRef.current?.seek(now);
    },
    loadHistorical: async (from: bigint, to: bigint) => {
      patch({ mode: "historical", rangeFrom: from, rangeTo: to, cursor: to, loop: null });
      await loadRange(from, to, { reset: true, topology: true });
      clientRef.current?.seek(to);
    },
    setFilters: (f: Filters) => clientRef.current?.setFilters(f),
    setViewport: (x: number, y: number, w: number, h: number, scale: number) =>
      clientRef.current?.setViewport(x, y, w, h, scale),
    pick: async (x: number, y: number) => (await clientRef.current?.call({ kind: "pick", x, y })) ?? null,
    nodeMetrics: async (nodeId: string, at?: bigint) =>
      (await clientRef.current?.call({ kind: "nodeMetrics", nodeId, atNs: at?.toString() })) ?? null,
    compare: async (a: bigint, b: bigint) =>
      (await clientRef.current?.call({ kind: "compare", aNs: a.toString(), bNs: b.toString() })) ?? [],
    diagnostics: async () => (await clientRef.current?.call({ kind: "diagnostics" })) ?? null,
    timelineTracks: async (from: bigint, to: bigint, buckets: number, clipCap: number) =>
      (await clientRef.current?.call({
        kind: "timelineTracks",
        fromNs: from.toString(),
        toNs: to.toString(),
        buckets,
        clipCap,
      })) ?? null,
    onFrame: (cb: (f: Frame) => void) => {
      const listeners = frameListeners.current;
      listeners.add(cb);
      if (lastFrameRef.current) cb(lastFrameRef.current);
      return () => {
        listeners.delete(cb);
      };
    },
    };
    return a;
  }, [seek, loadRange, patch]);

  return { state, actions };
}

export type WorkspaceActions = ReturnType<typeof useWorkspaceEngine>["actions"];
