import type { Frame, FrameSummary, WorkerRequest, WorkerResponse } from "@controlhub/web-engine";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EngineClient } from "./client";

class FakeWorker {
  static last: FakeWorker | null = null;
  readonly posted: WorkerRequest[] = [];
  terminated = false;
  onmessage: ((ev: MessageEvent<WorkerResponse>) => void) | null = null;
  onerror: ((ev: ErrorEvent) => void) | null = null;

  constructor(
    readonly url: URL,
    readonly options: WorkerOptions,
  ) {
    FakeWorker.last = this;
  }
  postMessage(msg: WorkerRequest) {
    this.posted.push(msg);
  }
  terminate() {
    this.terminated = true;
  }
  reply(msg: WorkerResponse) {
    this.onmessage?.({ data: msg } as MessageEvent<WorkerResponse>);
  }
  fail(message: string) {
    this.onerror?.({ message } as ErrorEvent);
  }
}

function worker(): FakeWorker {
  const w = FakeWorker.last;
  if (!w) throw new Error("no worker created");
  return w;
}

function idOf(msg: WorkerRequest | undefined): number {
  if (!msg || !("id" in msg)) throw new Error("request without id");
  return msg.id;
}

function frame(generation: number): Frame {
  const summary: FrameSummary = {
    generation,
    time_ns: null,
    prev_event_ns: null,
    next_event_ns: null,
    total_active: 0,
    window_requests: 0,
    window_errors: 0,
    visible_nodes: 0,
    visible_edges: 0,
    lod: 1,
    topology_revision: 0,
    hops_in_window: 0,
    hops_shown: 0,
  };
  return {
    nodeData: new Float32Array(0),
    nodeIndices: new Uint32Array(0),
    edgeData: new Float32Array(0),
    edgeIndices: new Uint32Array(0),
    changedNodes: new Uint32Array(0),
    hopData: new Float32Array(0),
    summary,
    computeMs: 0,
  };
}

beforeEach(() => {
  FakeWorker.last = null;
  vi.stubGlobal("Worker", FakeWorker);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("EngineClient", () => {
  it("starts a module worker", () => {
    new EngineClient();
    expect(worker().options).toEqual({ type: "module" });
    expect(worker().url.pathname).toMatch(/engine\.worker\.ts$/);
  });

  it("gives every RPC a unique id and resolves replies out of order", async () => {
    const c = new EngineClient();
    const a = c.call({ kind: "diagnostics" });
    const b = c.call({ kind: "pick", x: 1, y: 2 });
    const [ra, rb] = worker().posted;
    expect(ra).toEqual({ kind: "diagnostics", id: 1 });
    expect(rb).toEqual({ kind: "pick", x: 1, y: 2, id: 2 });
    worker().reply({ kind: "result", id: idOf(rb), value: "node-7" });
    worker().reply({ kind: "result", id: idOf(ra), value: { ok: 1 } });
    await expect(b).resolves.toBe("node-7");
    await expect(a).resolves.toEqual({ ok: 1 });
  });

  it("rejects on an error reply and ignores unknown or repeated ids", async () => {
    const c = new EngineClient();
    const p = c.call({ kind: "topology" });
    const id = idOf(worker().posted[0]);
    worker().reply({ kind: "result", id: 999, value: "stray" });
    worker().reply({ kind: "error", id, message: "engine not initialised" });
    worker().reply({ kind: "result", id, value: "late" });
    await expect(p).rejects.toThrow("engine not initialised");
  });

  it("sends seek time as a decimal string with an increasing generation", () => {
    const c = new EngineClient();
    const t = 1_790_000_000_123_456_789n;
    expect(c.seek(t)).toBe(1);
    expect(c.seek(t + 1n)).toBe(2);
    expect(worker().posted).toEqual([
      { kind: "seek", timeNs: "1790000000123456789", generation: 1 },
      { kind: "seek", timeNs: "1790000000123456790", generation: 2 },
    ]);
  });

  it("passes viewport, filters and hop window through without ids", () => {
    const c = new EngineClient();
    c.setViewport(1, 2, 3, 4, 0.5);
    c.setHopWindow(8_000_000_000n, 500_000_000n);
    expect(worker().posted).toEqual([
      { kind: "setViewport", x: 1, y: 2, width: 3, height: 4, scale: 0.5 },
      { kind: "setHopWindow", beforeNs: "8000000000", afterNs: "500000000" },
    ]);
  });

  it("drops frames older than the newest one seen", () => {
    const c = new EngineClient();
    const got: number[] = [];
    c.onFrame((f) => got.push(f.summary.generation));
    worker().reply({ kind: "frame", frame: frame(2) });
    worker().reply({ kind: "frame", frame: frame(1) });
    worker().reply({ kind: "frame", frame: frame(2) });
    worker().reply({ kind: "frame", frame: frame(3) });
    expect(got).toEqual([2, 2, 3]);
  });

  it("stops notifying unsubscribed listeners", () => {
    const c = new EngineClient();
    const fn = vi.fn();
    const off = c.onFrame(fn);
    off();
    worker().reply({ kind: "frame", frame: frame(1) });
    expect(fn).not.toHaveBeenCalled();
  });

  it("routes live and fatal messages to their listeners", () => {
    const c = new EngineClient();
    const live = vi.fn();
    const fatal = vi.fn();
    c.onLive(live);
    c.onFatal(fatal);
    worker().reply({ kind: "live", event: { type: "open" } });
    worker().reply({ kind: "fatal", message: "boom" });
    expect(live).toHaveBeenCalledWith({ type: "open" });
    expect(fatal).toHaveBeenCalledWith("boom");
  });

  it("fails all pending calls when the worker cannot start", async () => {
    const c = new EngineClient();
    const fatal = vi.fn();
    c.onFatal(fatal);
    const a = c.call({ kind: "init" });
    const b = c.call({ kind: "diagnostics" });
    worker().fail("");
    await expect(a).rejects.toThrow("engine worker failed to start");
    await expect(b).rejects.toThrow("engine worker failed to start");
    expect(fatal).toHaveBeenCalledWith("engine worker failed to start");
  });

  it("dispose terminates the worker, rejects pending calls and silences listeners", async () => {
    const c = new EngineClient();
    const fn = vi.fn();
    c.onFrame(fn);
    const p = c.call({ kind: "topology" });
    c.dispose();
    expect(worker().terminated).toBe(true);
    await expect(p).rejects.toThrow("engine disposed");
    worker().reply({ kind: "frame", frame: frame(5) });
    expect(fn).not.toHaveBeenCalled();
  });

  describe("settledFrame", () => {
    it("resolves at once when the last frame is for the latest seek", async () => {
      const c = new EngineClient();
      c.seek(1n);
      worker().reply({ kind: "frame", frame: frame(1) });
      await expect(c.settledFrame()).resolves.toMatchObject({ summary: { generation: 1 } });
    });

    it("waits for the frame of the latest seek, ignoring older ones", async () => {
      const c = new EngineClient();
      c.seek(1n);
      worker().reply({ kind: "frame", frame: frame(1) });
      c.seek(2n);
      const p = c.settledFrame();
      let settled = false;
      void p.then(() => (settled = true));
      await Promise.resolve();
      expect(settled).toBe(false);
      worker().reply({ kind: "frame", frame: frame(2) });
      await expect(p).resolves.toMatchObject({ summary: { generation: 2 } });
    });

    it("falls back to the last frame after the timeout", async () => {
      vi.useFakeTimers();
      const c = new EngineClient();
      c.seek(1n);
      worker().reply({ kind: "frame", frame: frame(1) });
      c.seek(2n);
      const p = c.settledFrame(50);
      vi.advanceTimersByTime(50);
      await expect(p).resolves.toMatchObject({ summary: { generation: 1 } });
    });

    it("resolves null after the timeout when no frame ever arrived", async () => {
      vi.useFakeTimers();
      const c = new EngineClient();
      c.seek(1n);
      const p = c.settledFrame(10);
      vi.advanceTimersByTime(10);
      await expect(p).resolves.toBeNull();
    });
  });
});
