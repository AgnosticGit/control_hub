"use client";

import type {
  Filters,
  Frame,
  LiveEvent,
  ResultMap,
  RpcKind,
  WorkerRequest,
  WorkerResponse,
} from "@controlhub/web-engine";

type RpcRequest<K extends RpcKind> = { kind: K } & Omit<Extract<WorkerRequest, { kind: K }>, "id" | "kind">;

interface Pending {
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
}

/** UI-thread handle to the engine Worker. */
export class EngineClient {
  private readonly worker: Worker;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly frameListeners = new Set<(f: Frame) => void>();
  private readonly fatalListeners = new Set<(message: string) => void>();
  private readonly liveListeners = new Set<(event: LiveEvent) => void>();
  private generation = 0;
  private lastFrameGeneration = -1;
  private lastFrame: Frame | null = null;

  constructor() {
    this.worker = new Worker(new URL("./engine.worker.ts", import.meta.url), { type: "module" });
    this.worker.onmessage = (ev: MessageEvent<WorkerResponse>) => this.receive(ev.data);
    this.worker.onerror = (ev) => {
      const message = ev.message || "engine worker failed to start";
      for (const l of this.fatalListeners) l(message);
      for (const p of this.pending.values()) p.reject(new Error(message));
      this.pending.clear();
    };
  }

  private receive(msg: WorkerResponse) {
    switch (msg.kind) {
      case "result":
      case "error": {
        const p = this.pending.get(msg.id);
        if (!p) return;
        this.pending.delete(msg.id);
        if (msg.kind === "result") p.resolve(msg.value);
        else p.reject(new Error(msg.message));
        return;
      }
      case "frame": {
        const g = Number(msg.frame.summary.generation);
        if (g < this.lastFrameGeneration) return;
        this.lastFrameGeneration = g;
        this.lastFrame = msg.frame;
        for (const l of this.frameListeners) l(msg.frame);
        return;
      }
      case "live":
        for (const l of this.liveListeners) l(msg.event);
        return;
      case "fatal":
        for (const l of this.fatalListeners) l(msg.message);
        return;
    }
  }

  call<K extends RpcKind>(req: RpcRequest<K>): Promise<ResultMap[K]> {
    const id = this.nextId++;
    return new Promise<ResultMap[K]>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.worker.postMessage({ ...req, id } as WorkerRequest);
    });
  }

  /** Requests state at `timeNs`; returns the generation that will identify the resulting frame. */
  seek(timeNs: bigint): number {
    this.generation += 1;
    this.worker.postMessage({ kind: "seek", timeNs: timeNs.toString(), generation: this.generation } satisfies WorkerRequest);
    return this.generation;
  }

  setViewport(x: number, y: number, width: number, height: number, scale: number) {
    this.worker.postMessage({ kind: "setViewport", x, y, width, height, scale } satisfies WorkerRequest);
  }

  setFilters(filters: Filters) {
    this.worker.postMessage({ kind: "setFilters", filters } satisfies WorkerRequest);
  }

  setHopWindow(beforeNs: bigint, afterNs: bigint) {
    this.worker.postMessage({
      kind: "setHopWindow",
      beforeNs: beforeNs.toString(),
      afterNs: afterNs.toString(),
    } satisfies WorkerRequest);
  }

  /**
   * Resolves with the frame for the most recent seek (waiting for it if needed), so decisions
   * such as "step to next event" never use a summary computed for an older cursor.
   * Falls back to the last frame after `timeoutMs`.
   */
  settledFrame(timeoutMs = 2000): Promise<Frame | null> {
    const target = this.generation;
    if (this.lastFrame && this.lastFrameGeneration >= target) return Promise.resolve(this.lastFrame);
    return new Promise((resolve) => {
      const done = (f: Frame | null) => {
        clearTimeout(timer);
        off();
        resolve(f);
      };
      const off = this.onFrame((f) => {
        if (Number(f.summary.generation) >= target) done(f);
      });
      const timer = setTimeout(() => done(this.lastFrame), timeoutMs);
    });
  }

  onFrame(cb: (f: Frame) => void): () => void {
    this.frameListeners.add(cb);
    return () => this.frameListeners.delete(cb);
  }

  onLive(cb: (event: LiveEvent) => void): () => void {
    this.liveListeners.add(cb);
    return () => this.liveListeners.delete(cb);
  }

  onFatal(cb: (message: string) => void): () => void {
    this.fatalListeners.add(cb);
    return () => this.fatalListeners.delete(cb);
  }

  dispose() {
    this.worker.terminate();
    for (const p of this.pending.values()) p.reject(new Error("engine disposed"));
    this.pending.clear();
    this.frameListeners.clear();
    this.fatalListeners.clear();
    this.liveListeners.clear();
  }
}
