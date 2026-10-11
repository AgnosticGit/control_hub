"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { EdgeChange, Filters, Frame, FrameSummary, InitResult, InfrastructureSnapshot, LiveSignal, LoadResult, TopologyView } from "@karbird/web-engine";
import type { Schemas } from "@/lib/api";
import { EngineClient } from "@/engine/client";
import { scopedUrl } from "@/lib/api";
import { NS_PER_MS, NS_PER_SEC, clampNs, nowNs, parseNs } from "@/lib/time";
import type { AnimClock } from "@/renderer/hops";
import { firstItemStart, playStart, replayStep } from "./timeline/range";
import { clampRangeToNow, hasFuture, rangeCoveredBy } from "./timeline/scale";
import { asLogSource, type LogSource } from "./logs-query";
import { ENGINE_ROLLING_WINDOW_NS } from "./service-card-metrics";

export type Mode = "live" | "paused" | "replay" | "historical";
export const SPEED_MIN = 0.001;
export const SPEED_MAX = 10;
export const SPEED_SLIDER_STEPS = 1000;

/** Logarithmic slider: 250 fine steps per tenfold change in replay speed. */
export function speedFromSlider(position: number): number {
  const step = Math.max(0, Math.min(SPEED_SLIDER_STEPS, Math.round(Number.isFinite(position) ? position : 0)));
  return SPEED_MIN * 10 ** (step / 250);
}

export function sliderPositionForSpeed(speed: number): number {
  const safeSpeed = Math.max(SPEED_MIN, Math.min(SPEED_MAX, Number.isFinite(speed) && speed > 0 ? speed : SPEED_MIN));
  return Math.round(Math.log10(safeSpeed / SPEED_MIN) * 250);
}

export function formatSpeed(speed: number): string {
  return Number(speed.toPrecision(4)).toString();
}

/** Replay speeds; 0.001× shows one millisecond of recorded time per second. */
export const SPEEDS = Array.from({ length: SPEED_SLIDER_STEPS + 1 }, (_, position) => speedFromSlider(position));
export type Speed = number;

/**
 * Live request animation runs this far behind the wall clock: SDKs export spans in batches
 * (OpenTelemetry's default is every 5 s), so the newest requests are not loaded yet at "now".
 */
export const LIVE_ANIM_DELAY_MS = 8000;

export function infrastructureFeedQuery(enabled: boolean): { infrastructure: "true" } | Record<string, never> {
  return enabled ? { infrastructure: "true" } : {};
}

export function animClockFor(mode: Mode, speed: Speed): AnimClock {
  if (mode === "live") return { kind: "wall", delayMs: LIVE_ANIM_DELAY_MS };
  return { kind: "frame", rate: mode === "replay" ? speed : 0 };
}

/** A seek past the loaded data of a range that is still growing: it lands in the future. */
export function seeksIntoFuture(t: bigint, rangeTo: bigint, mode: Mode): boolean {
  return t > rangeTo && hasFuture(mode);
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
/** Live clock: advances the playhead locally; telemetry itself arrives over the WebSocket. */
const LIVE_TICK_MS = 2000;
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
  /** Where the timeline's log items came from; unset for live batches (always the hot store). */
  logsSource?: LogSource;
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
  liveTransport: "off" | "connecting" | "websocket" | "stale";
}

export function asCompleteness(v: unknown): Completeness | null {
  return v && typeof v === "object" && "processing_caught_up" in v ? (v as Completeness) : null;
}

export function useWorkspaceEngine(projectId: string, environmentId: string) {
  const clientRef = useRef<EngineClient | null>(null);
  const infrastructureLayoutEnabledRef = useRef(false);
  const infrastructureFeedEnabledRef = useRef(false);
  /** Settles once every range load started so far is done (success or not). */
  const loadsRef = useRef<Promise<void>>(Promise.resolve());
  const loadedChunksRef = useRef<Array<{ from: bigint; to: bigint }>>([]);
  const chunkLoadsRef = useRef(new Map<string, Promise<void>>());
  const frameListeners = useRef(new Set<(f: Frame) => void>());
  const liveSignalListeners = useRef(new Set<(signals: LiveSignal[]) => void>());
  const infrastructureListeners = useRef(new Set<(snapshot: InfrastructureSnapshot) => void>());
  const actionPlanListeners = useRef(new Set<(plan: Schemas["ActionPlanDto"]) => void>());
  const edgeChangeListeners = useRef(new Set<(change: EdgeChange) => void>());
  const liveActionPlans = useRef(new Map<string, Schemas["ActionPlanDto"]>());
  const recentLiveSignals = useRef<LiveSignal[]>([]);
  const latestInfrastructure = useRef<InfrastructureSnapshot | null>(null);
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
  // Mirrors `state` synchronously so callbacks (rAF loop, timers) never read a
  // value older than the last patch.
  const stateRef = useRef(state);

  useEffect(() => {
    liveActionPlans.current.clear();
    recentLiveSignals.current = [];
    latestInfrastructure.current = null;
    loadedChunksRef.current = [];
    chunkLoadsRef.current.clear();
  }, [projectId, environmentId]);

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
    const topology = await c.call({ kind: "topology" });
    if (c === clientRef.current && topology.revision >= (stateRef.current.topology?.revision ?? -1)) patch({ topology });
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
          logsSource: asLogSource(r.logsSource),
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
      let done = () => {};
      const settled = new Promise<void>((resolve) => {
        done = resolve;
      });
      const before = loadsRef.current;
      loadsRef.current = Promise.all([before, settled]).then(() => undefined);
      patch({ loading: true });
      try {
        if (opts.reset) {
          loadedChunksRef.current = [];
          await c.call({ kind: "evictBefore", beforeNs: EVICT_ALL.toString() });
        }
        if (opts.topology) {
          await c.call({ kind: "loadTopology", url: url("topology", from, to, { ...(stateRef.current.mode === "historical" ? {} : { live: "true" }),...(infrastructureLayoutEnabledRef.current ? {at:stateRef.current.cursor.toString()}: {}) }) });
        }
        const r = await c.call({ kind: "loadTimeline", url: url("timeline", from, to, { limit: TIMELINE_LIMIT }) });
        loadedChunksRef.current.push({ from, to });
        recordLoad(r);
        if (opts.topology) await refreshTopologyView();
        patch({ error: null });
      } catch (e) {
        patch({ error: e instanceof Error ? e.message : String(e) });
      } finally {
        patch({ loading: false });
        done();
      }
    },
    [patch, url, recordLoad, refreshTopologyView],
  );

  const loadTimelineChunk = useCallback(async (from: bigint, to: bigint) => {
    const bounded = clampRangeToNow({ from, to }, nowNs());
    if (!bounded || bounded.to - bounded.from > 7n * 86_400n * NS_PER_SEC) return;
    const boundedFrom = bounded.from;
    const boundedTo = bounded.to;
    const key = `${boundedFrom}:${boundedTo}`;
    const pending = chunkLoadsRef.current.get(key);
    if (pending) return pending;
    if (loadedChunksRef.current.some((range) => range.from <= boundedFrom && range.to >= boundedTo)) return;
    const c = clientRef.current;
    if (!c) return;

    const task = (async () => {
      void c.call({ kind: "liveStop" });
      patch({ mode: "paused", loading: true, error: null });
      try {
        await c.call({
          kind: "loadTopology",
          url: url("topology", boundedFrom, boundedTo, {
            live: "true",
            ...(infrastructureLayoutEnabledRef.current ? { at: stateRef.current.cursor.toString() } : {}),
          }),
        });
        const result = await c.call({ kind: "loadTimeline", url: url("timeline", boundedFrom, boundedTo, { limit: TIMELINE_LIMIT }) });
        if (c !== clientRef.current) return;
        loadedChunksRef.current.push({ from: boundedFrom, to: boundedTo });
        recordLoad(result);
        await refreshTopologyView();
        const s = stateRef.current;
        patch({ rangeFrom: boundedFrom < s.rangeFrom ? boundedFrom : s.rangeFrom, rangeTo: boundedTo > s.rangeTo ? boundedTo : s.rangeTo });
        c.seek(s.cursor);
      } catch (error) {
        if (c === clientRef.current) patch({ error: error instanceof Error ? error.message : String(error) });
      } finally {
        if (c === clientRef.current) patch({ loading: false });
        chunkLoadsRef.current.delete(key);
      }
    })();
    chunkLoadsRef.current.set(key, task);
    return task;
  }, [patch, recordLoad, refreshTopologyView, url]);

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
    recentLiveSignals.current = [];
    latestInfrastructure.current = null;
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
      if (f.topology) {
        patch({ topology: f.topology });
      } else if (lastRevision !== null && lastRevision !== f.summary.topology_revision) {
        void client.call({ kind: "topology" }).then((topology) => {
          if (client === clientRef.current && topology.revision >= (stateRef.current.topology?.revision ?? -1)) patch({ topology });
        }).catch(() => {});
      }
      lastRevision = f.summary.topology_revision;
      const wait = SUMMARY_MS - (performance.now() - lastSummaryAt);
      if (wait <= 0) publish();
      else if (!trailing) trailing = setTimeout(publish, wait);
    });
    const offFatal = client.onFatal((message) => patch({ phase: "error", error: message }));
    (async () => {
      try {
        const init = await client.call({ kind: "init", config:{server_owned_topology:true, rolling_window_ns: ENGINE_ROLLING_WINDOW_NS.toString()} });
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
  // A failed socket marks the view stale and reconnects with backoff; it never polls HTTP.
  useEffect(() => {
    if (state.phase !== "ready" || state.mode !== "live") return;
    const c = clientRef.current;
    if (!c) return;
    let cancelled = false;
    let busy = false;
    let lastEvict = Date.now();

    const tick = async () => {
      if (cancelled || busy) return;
      busy = true;
      try {
        const now = nowNs();
        if (Date.now() - lastEvict > EVICT_EVERY_MS) {
          lastEvict = Date.now();
          await c.call({ kind: "evictBefore", beforeNs: (now - LIVE_WINDOW_NS).toString() });
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
          patch({ liveTransport: "stale" });
          return;
        case "signal": {
          recentLiveSignals.current = [...recentLiveSignals.current, ...ev.signals].slice(-2000);
          for (const listener of liveSignalListeners.current) listener(ev.signals);
          const prev = stateRef.current.load;
          patch({
            liveTransport: "websocket",
            load: {
              items: 1,
              fetchMs: 0,
              applyMs: ev.applyMs,
              truncated: false,
              completeness: prev?.completeness ?? null,
              at: Date.now(),
            },
          });
          return;
        }
        case "batch": {
          if (ev.signals.length) {
            recentLiveSignals.current = [...recentLiveSignals.current, ...ev.signals].slice(-2000);
            for (const listener of liveSignalListeners.current) listener(ev.signals);
          }
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
        case "infrastructure":
          latestInfrastructure.current = ev.snapshot;
          for (const listener of infrastructureListeners.current) listener(ev.snapshot);
          return;
        case "action": {
          const plan = ev.plan;
          if (typeof plan.id !== "string" || typeof plan.state !== "string") return;
          const dto = plan as Schemas["ActionPlanDto"];
          liveActionPlans.current.set(dto.id, dto);
          for (const listener of actionPlanListeners.current) listener(dto);
          return;
        }
        case "edge_change":
          for (const listener of edgeChangeListeners.current) listener(ev.change);
          return;
        case "gap": {
          patch({ liveTransport: "stale", error: "Live data needs recovery; reconnecting to the stream." });
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
    void c.call({ kind: "liveStart", url: scopedUrl(projectId, environmentId, "live", infrastructureFeedQuery(infrastructureFeedEnabledRef.current)) });
    const id = setInterval(() => void tick(), LIVE_TICK_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
      offLive();
      void c.call({ kind: "liveStop" });
      patch({ liveTransport: "off" });
    };
  }, [state.phase, state.mode, patch, projectId, environmentId]);

  useEffect(() => {
    if (state.phase !== "ready") return;
    const w = hopWindowFor(state.mode);
    clientRef.current?.setHopWindow(w.before, w.after);
  }, [state.phase, state.mode]);

  // Historical cache refreshes can finish after the initial response. Adopt the completed
  // revision and late telemetry while keeping the user's historical cursor unchanged.
  useEffect(() => {
    if (state.phase !== "ready" || state.mode === "live") return;
    let cancelled=false;
    let busy=false;
    const timer=setInterval(()=>{
      const client=clientRef.current;
      if(cancelled || busy || !client) return;
      busy=true;
      const {rangeFrom,rangeTo}=stateRef.current;
      void (async()=>{
        try {
          await client.call({kind:"loadTopology",url:url("topology",rangeFrom,rangeTo,{...(stateRef.current.mode==="historical" ? {} : {live:"true"}),...(infrastructureLayoutEnabledRef.current ? {at:stateRef.current.cursor.toString()}: {})})});
          if(cancelled || client!==clientRef.current) return;
          const result=await client.call({kind:"loadTimeline",url:url("timeline",rangeFrom,rangeTo,{limit:TIMELINE_LIMIT})});
          if(!cancelled && client===clientRef.current) {
            recordLoad(result);
            client.seek(stateRef.current.cursor);
          }
        } catch(error) {
          if(!cancelled) patch({error:error instanceof Error ? error.message : String(error)});
        } finally {busy=false;}
      })();
    },TOPOLOGY_POLL_MS);
    return ()=>{cancelled=true;clearInterval(timer);};
  },[state.phase,state.mode,state.rangeFrom,state.rangeTo,url,recordLoad,patch]);

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
    play: async () => {
      const s = stateRef.current;
      const start = playStart(s.cursor, s.rangeFrom, s.rangeTo, s.loop);
      if (!s.loop && start !== s.cursor) {
        // Starting over: skip the empty stretch before the first recorded item, once it is loaded.
        await loadsRef.current;
        const first = await clientRef.current
          ?.call({ kind: "nextEvent", afterNs: (s.rangeFrom - 1n).toString() })
          .catch(() => null);
        const now = stateRef.current;
        seek(firstItemStart(parseNs(first), now.rangeFrom, now.rangeTo));
      } else if (start !== s.cursor) {
        seek(start);
      }
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
      if (seeksIntoFuture(t, s.rangeTo, s.mode)) {
        // Nothing to show after the loaded data: moving there means "back to now".
        if (s.mode !== "live") void a.returnToLive();
        return;
      }
      if (s.mode === "live" || s.mode === "replay") patch({ mode: "paused" });
      seek(t);
    },
    // From the cursor itself: the last frame may still describe an earlier seek (e.g. Home, then Next).
    // After the range loads: before that the engine only holds what live streaming brought.
    stepPrev: async () => {
      await loadsRef.current;
      const t = await clientRef.current
        ?.call({ kind: "prevEvent", beforeNs: stateRef.current.cursor.toString() })
        .catch(() => null);
      const p = parseNs(t);
      if (p !== null) a.seek(p);
    },
    stepNext: async () => {
      await loadsRef.current;
      const t = await clientRef.current
        ?.call({ kind: "nextEvent", afterNs: stateRef.current.cursor.toString() })
        .catch(() => null);
      const n = parseNs(t);
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
    loadTimelineChunk,
    hasTimelineRange: (from: bigint, to: bigint) => rangeCoveredBy(loadedChunksRef.current, { from, to }),
    /** Machines, instances and saved backend placement at the selected time. */
    loadInfrastructure: async (at: bigint, live = false) =>
      (await clientRef.current?.call({
        kind: "loadInfrastructure",
        url: scopedUrl(projectId, environmentId, "infrastructure", { at: at.toString(), ...(live ? { live: "true" } : {}) }),
      })) ?? null,
    setFilters: (f: Filters) => {
      infrastructureLayoutEnabledRef.current = f.infrastructure;
      clientRef.current?.setFilters(f);
    },
    setInfrastructureFeedEnabled: (enabled: boolean) => {
      if (infrastructureFeedEnabledRef.current === enabled) return;
      infrastructureFeedEnabledRef.current = enabled;
      if (stateRef.current.mode === "live") {
        const client = clientRef.current;
        if (client) void client.call({ kind: "liveStart", url: scopedUrl(projectId, environmentId, "live", infrastructureFeedQuery(enabled)) });
      }
    },
    onLiveSignals: (listener: (signals: LiveSignal[]) => void) => {
      liveSignalListeners.current.add(listener);
      if (recentLiveSignals.current.length) listener(recentLiveSignals.current);
      return () => { liveSignalListeners.current.delete(listener); };
    },
    onInfrastructure: (listener: (snapshot: InfrastructureSnapshot) => void) => {
      infrastructureListeners.current.add(listener);
      if (latestInfrastructure.current) listener(latestInfrastructure.current);
      return () => { infrastructureListeners.current.delete(listener); };
    },
    onActionPlan: (listener: (plan: Schemas["ActionPlanDto"]) => void) => {
      actionPlanListeners.current.add(listener);
      for (const plan of liveActionPlans.current.values()) listener(plan);
      return () => { actionPlanListeners.current.delete(listener); };
    },
    onEdgeChange: (listener: (change: EdgeChange) => void) => {
      edgeChangeListeners.current.add(listener);
      return () => { edgeChangeListeners.current.delete(listener); };
    },
    subscribeEdge: (source: string, target: string, relation: string, afterCursorNs: string) =>
      clientRef.current?.call({ kind: "subscribeEdge", source, target, relation, afterCursorNs }),
    unsubscribeEdge: () => clientRef.current?.call({ kind: "unsubscribeEdge" }),
    setViewport: (x: number, y: number, w: number, h: number, scale: number) =>
      clientRef.current?.setViewport(x, y, w, h, scale),
    pick: async (x: number, y: number) => (await clientRef.current?.call({ kind: "pick", x, y })) ?? null,
    nodeMetrics: async (nodeId: string, at?: bigint) =>
      (await clientRef.current?.call({ kind: "nodeMetrics", nodeId, atNs: at?.toString() })) ?? null,
    edgeMetrics: async (sourceId: string, targetId: string, at?: bigint) =>
      (await clientRef.current?.call({ kind: "edgeMetrics", sourceId, targetId, atNs: at?.toString() })) ?? null,
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
  }, [seek, loadRange, loadTimelineChunk, patch, projectId, environmentId]);

  return { state, actions };
}

export type WorkspaceActions = ReturnType<typeof useWorkspaceEngine>["actions"];
