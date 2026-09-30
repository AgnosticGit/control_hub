import type { TimelineTracks } from "@controlhub/web-engine";
import { describe, expect, it } from "vitest";
import { FUTURE_EDGE, FUTURE_FILL, RULER_H, actionColor, bucketGeometry, draw, drawRuler, rowTop, totalHeight, withAlpha, type DrawInput, type Row } from "./draw";
import type { View } from "./scale";

interface Call {
  name: string;
  args: unknown[];
  fillStyle: unknown;
  globalAlpha: number;
}

/** Records every method call together with the fill style and alpha in effect at that moment. */
function fakeCtx() {
  const calls: Call[] = [];
  const state: Record<string | symbol, unknown> = { globalAlpha: 1, fillStyle: "#000" };
  const ctx = new Proxy(state, {
    get(target, prop) {
      if (prop in target) return target[prop];
      return (...args: unknown[]) => {
        calls.push({ name: String(prop), args, fillStyle: target.fillStyle, globalAlpha: Number(target.globalAlpha) });
      };
    },
    set(target, prop, value) {
      target[prop] = value;
      return true;
    },
  }) as unknown as CanvasRenderingContext2D;
  return { ctx, calls, named: (name: string) => calls.filter((c) => c.name === name) };
}

const VIEW: View = { from: 0n, to: 1_000_000_000n };
const WIDTH = 1000;

function tracks(p: Partial<TimelineTracks> = {}): TimelineTracks {
  return {
    from_ns: "0",
    to_ns: "1000000000",
    buckets: 10,
    requests: [],
    errors: [],
    events: [],
    latency_p95_ms: [],
    latency_samples: [],
    services: [],
    clips: [],
    markers: [],
    markers_total: 0,
    ...p,
  };
}

function input(p: Partial<DrawInput> = {}): DrawInput {
  return {
    width: WIDTH,
    rows: [],
    view: VIEW,
    tracks: null,
    actions: [],
    cursor: 500_000_000n,
    markA: null,
    hoverX: null,
    range: null,
    ...p,
  };
}

const row = (id: string, kind: Row["kind"], height = 20, extra: Partial<Row> = {}): Row => ({ id, label: id, kind, height, ...extra });

describe("layout helpers", () => {
  const rows = [row("a", "actions", 10), row("b", "events", 20), row("c", "service", 30)];

  it("rowTop sums the heights above a row", () => {
    expect(rowTop(rows, 0)).toBe(0);
    expect(rowTop(rows, 1)).toBe(10);
    expect(rowTop(rows, 2)).toBe(30);
    expect(rowTop(rows, 99)).toBe(60);
  });

  it("totalHeight sums all rows", () => {
    expect(totalHeight(rows)).toBe(60);
    expect(totalHeight([])).toBe(0);
  });
});

describe("bucketGeometry", () => {
  it("maps buckets to pixels in the current view", () => {
    const g = bucketGeometry(tracks(), VIEW, WIDTH);
    expect(g.from).toBe(0n);
    expect(g.x0).toBe(0);
    expect(g.bucketNs).toBe(100_000_000);
    expect(g.pxPerNs).toBeCloseTo(1e-6);
    expect(g.bucketPx).toBeCloseTo(100);
  });

  it("follows a zoomed/panned view", () => {
    const g = bucketGeometry(tracks(), { from: 500_000_000n, to: 1_000_000_000n }, WIDTH);
    expect(g.x0).toBe(-1000);
    expect(g.bucketPx).toBeCloseTo(200);
  });

  it("does not divide by zero", () => {
    const g = bucketGeometry(tracks({ buckets: 0 }), { from: 5n, to: 5n }, WIDTH);
    expect(Number.isFinite(g.bucketNs)).toBe(true);
    expect(Number.isFinite(g.pxPerNs)).toBe(true);
  });
});

describe("actionColor", () => {
  it("colours plans by state", () => {
    expect(actionColor("succeeded")).toBe("#22c55e");
    expect(actionColor("failed")).toBe("#ef4444");
    expect(actionColor("verification_failed")).toBe("#ef4444");
    expect(actionColor("running")).toBe("#f59e0b");
    expect(actionColor("approved")).toBe("#f59e0b");
    expect(actionColor("pending_approval")).toBe("#7d8aa5");
    expect(actionColor("")).toBe("#7d8aa5");
  });
});

describe("draw", () => {
  it("handles no rows and no tracks", () => {
    const { ctx, named } = fakeCtx();
    expect(() => draw(ctx, input())).not.toThrow();
    expect(named("clearRect")[0]?.args).toEqual([0, 0, WIDTH, 0]);
  });

  it("draws every row kind without tracks", () => {
    const { ctx } = fakeCtx();
    const rows = (["actions", "events", "errors", "requests", "latency", "group", "service"] as const).map((k) => row(k, k, 20, { track: 0 }));
    expect(() => draw(ctx, input({ rows }))).not.toThrow();
  });

  it("draws every row kind with empty tracks and a missing service track", () => {
    const { ctx } = fakeCtx();
    const rows = [row("s", "service", 20, { track: 5 }), row("l", "latency"), row("e", "events")];
    expect(() => draw(ctx, input({ rows, tracks: tracks() }))).not.toThrow();
  });

  it("shades alternate rows", () => {
    const { ctx, calls } = fakeCtx();
    draw(ctx, input({ rows: [row("a", "group", 10), row("b", "group", 15), row("c", "group", 20)] }));
    const alt = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "rgba(16, 26, 46, 0.45)");
    expect(alt.map((c) => c.args)).toEqual([[0, 10, WIDTH, 15]]);
  });

  it("draws bars scaled to the maximum bucket", () => {
    const { ctx, calls } = fakeCtx();
    draw(ctx, input({ rows: [row("r", "requests", 25)], tracks: tracks({ requests: [0, 5, 10] }) }));
    const bars = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "#3b82f6");
    expect(bars).toHaveLength(2);
    const heights = bars.map((b) => b.args[3]);
    expect(heights).toEqual([10, 20]);
  });

  it("draws dimmed rows faded and restores full alpha afterwards", () => {
    const { ctx, calls } = fakeCtx();
    const t = tracks({ services: [{ node: 0, name: "orders", counts: [1, 2], spans_in_view: 3, clipped: false }] });
    draw(ctx, input({ rows: [row("s", "service", 20, { track: 0, dimmed: true })], tracks: t, cursor: -10n }));
    const heat = calls.filter((c) => c.name === "fillRect" && String(c.fillStyle).startsWith("rgba(59, 130, 246,"));
    expect(heat).toHaveLength(2);
    for (const h of heat) expect(h.globalAlpha).toBeCloseTo(0.28);
    expect(ctx.globalAlpha).toBe(1);
  });

  it("draws active rows at full alpha", () => {
    const { ctx, calls } = fakeCtx();
    const t = tracks({ services: [{ node: 0, name: "orders", counts: [4], spans_in_view: 4, clipped: false }] });
    draw(ctx, input({ rows: [row("s", "service", 20, { track: 0 })], tracks: t }));
    const heat = calls.filter((c) => c.name === "fillRect" && String(c.fillStyle).startsWith("rgba(59, 130, 246,"));
    expect(heat[0]?.globalAlpha).toBe(1);
  });

  it("draws span clips with their names for clipped services", () => {
    const { ctx, named } = fakeCtx();
    const t = tracks({
      services: [{ node: 0, name: "orders", counts: [], spans_in_view: 1, clipped: true }],
      clips: [
        { track: 0, start_ms: 100, end_ms: 400, depth: 0, error: false, name: "GET /orders", trace_id: "t" },
        { track: 1, start_ms: 0, end_ms: 1, depth: 0, error: true, name: "other track", trace_id: "t" },
      ],
    });
    draw(ctx, input({ rows: [row("s", "service", 30, { track: 0 })], tracks: t }));
    expect(named("fillText").map((c) => c.args[0])).toEqual(["GET /orders"]);
  });

  it("draws a service row in that service's colour, failed spans in red", () => {
    const heatCtx = fakeCtx();
    const heatTracks = tracks({ services: [{ node: 0, name: "orders", counts: [2, 4], spans_in_view: 6, clipped: false }] });
    draw(heatCtx.ctx, input({ rows: [row("s", "service", 20, { track: 0, color: "#a78bfa" })], tracks: heatTracks }));
    const heat = heatCtx.calls.filter((c) => c.name === "fillRect" && String(c.fillStyle).startsWith("rgba(167, 139, 250,"));
    expect(heat).toHaveLength(2);

    const clipCtx = fakeCtx();
    const clipTracks = tracks({
      services: [{ node: 0, name: "orders", counts: [], spans_in_view: 2, clipped: true }],
      clips: [
        { track: 0, start_ms: 100, end_ms: 400, depth: 0, error: false, name: "ok", trace_id: "t" },
        { track: 0, start_ms: 500, end_ms: 600, depth: 1, error: true, name: "bad", trace_id: "t" },
      ],
    });
    draw(clipCtx.ctx, input({ rows: [row("s", "service", 30, { track: 0, color: "#a78bfa" })], tracks: clipTracks }));
    const fills = clipCtx.named("fill").map((c) => c.fillStyle);
    expect(fills).toContain("rgba(167, 139, 250, 0.35)");
    expect(fills).toContain("rgba(239, 68, 68, 0.2)");
  });

  it("withAlpha converts #rrggbb", () => {
    expect(withAlpha("#3b82f6", 0.5)).toBe("rgba(59, 130, 246, 0.5)");
    expect(withAlpha("#000000", 1)).toBe("rgba(0, 0, 0, 1)");
  });

  it("splits the latency line at empty buckets", () => {
    const { ctx, named } = fakeCtx();
    draw(ctx, input({ rows: [row("l", "latency", 30)], tracks: tracks({ latency_p95_ms: [5, 10, null, 8, null, null, 3] }) }));
    expect(named("stroke")).toHaveLength(3);
  });

  it("draws action bars in their state colour with a label when wide enough", () => {
    const { ctx, calls, named } = fakeCtx();
    draw(
      ctx,
      input({
        rows: [row("a", "actions", 20)],
        actions: [
          { id: "1", title: "Restart", target: "orders", state: "succeeded", from: 100_000_000n, to: 300_000_000n },
          { id: "2", title: "Scale", target: "x", state: "failed", from: 2_000_000_000n, to: 3_000_000_000n },
        ],
      }),
    );
    const fills = calls.filter((c) => c.name === "fill");
    expect(fills.map((f) => f.fillStyle)).toContain("#22c55e");
    expect(fills.map((f) => f.fillStyle)).not.toContain("#ef4444");
    expect(named("fillText").map((c) => c.args[0])).toEqual(["Restart · orders"]);
  });

  it("shades the time outside the replay range", () => {
    const { ctx, calls } = fakeCtx();
    const rows = [row("g", "group", 40)];
    draw(ctx, input({ rows, range: { from: 250_000_000n, to: 750_000_000n }, cursor: -10n }));
    const shade = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "rgba(6, 10, 19, 0.55)");
    expect(shade.map((c) => c.args)).toEqual([
      [0, 0, 250, 40],
      [750, 0, 250, 40],
    ]);
    const edges = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "rgba(34, 211, 238, 0.8)");
    expect(edges.map((c) => c.args[0])).toEqual([249.5, 749.5]);
  });

  it("does not shade when the range covers the whole view", () => {
    const { ctx, calls } = fakeCtx();
    draw(ctx, input({ rows: [row("g", "group", 40)], range: { from: -1n, to: 2_000_000_000n } }));
    expect(calls.filter((c) => c.fillStyle === "rgba(6, 10, 19, 0.55)" && c.name === "fillRect")).toHaveLength(0);
  });

  it("draws the cursor only when it is in view", () => {
    const inView = fakeCtx();
    draw(inView.ctx, input({ rows: [row("g", "group", 40)], cursor: 500_000_000n }));
    expect(inView.calls.some((c) => c.name === "fillRect" && c.fillStyle === "#22d3ee")).toBe(true);
    const outside = fakeCtx();
    draw(outside.ctx, input({ rows: [row("g", "group", 40)], cursor: 5_000_000_000n }));
    expect(outside.calls.some((c) => c.name === "fillRect" && c.fillStyle === "#22d3ee")).toBe(false);
  });

  it("tints the time after the data ends and marks the edge", () => {
    const { ctx, calls } = fakeCtx();
    draw(ctx, input({ rows: [row("g", "group", 40)], future: 800_000_000n, cursor: 900_000_000n }));
    const tint = calls.filter((c) => c.name === "fillRect" && c.fillStyle === FUTURE_FILL);
    expect(tint.map((c) => c.args)).toEqual([[800, 0, 200, 40]]);
    const edge = calls.filter((c) => c.name === "fillRect" && c.fillStyle === FUTURE_EDGE);
    expect(edge.length).toBeGreaterThan(0);
    for (const e of edge) expect(e.args[0]).toBe(800);
    expect(calls.some((c) => c.name === "fillRect" && c.fillStyle === "#22d3ee")).toBe(true);
  });

  it("tints the whole view when it lies after the data, and nothing when it lies before", () => {
    const after = fakeCtx();
    draw(after.ctx, input({ rows: [row("g", "group", 40)], future: -5n }));
    expect(after.calls.filter((c) => c.name === "fillRect" && c.fillStyle === FUTURE_FILL).map((c) => c.args)).toEqual([[0, 0, WIDTH, 40]]);
    expect(after.calls.some((c) => c.fillStyle === FUTURE_EDGE)).toBe(false);

    for (const future of [null, undefined, 2_000_000_000n]) {
      const none = fakeCtx();
      draw(none.ctx, input({ rows: [row("g", "group", 40)], future }));
      expect(none.calls.some((c) => c.fillStyle === FUTURE_FILL || c.fillStyle === FUTURE_EDGE)).toBe(false);
    }
  });
});

describe("drawRuler", () => {
  it("draws ticks, mark A and the playhead", () => {
    const { ctx, named } = fakeCtx();
    drawRuler(ctx, { width: WIDTH, view: VIEW, cursor: 500_000_000n, markA: 100_000_000n, hoverX: 10, range: null });
    expect(named("clearRect")[0]?.args).toEqual([0, 0, WIDTH, RULER_H]);
    const labels = named("fillText").map((c) => c.args[0]);
    expect(labels).toContain("A");
    expect(labels.length).toBeGreaterThan(2);
  });

  it("omits mark A outside the view", () => {
    const { ctx, named } = fakeCtx();
    drawRuler(ctx, { width: WIDTH, view: VIEW, cursor: 0n, markA: 5_000_000_000n, hoverX: null, range: { from: 0n, to: 1n } });
    expect(named("fillText").map((c) => c.args[0])).not.toContain("A");
  });

  it("tints the future on the ruler too and keeps tick labels out of it", () => {
    const { ctx, calls, named } = fakeCtx();
    drawRuler(ctx, { width: WIDTH, view: VIEW, cursor: 0n, markA: null, hoverX: null, range: null, future: 750_000_000n });
    expect(calls.filter((c) => c.name === "fillRect" && c.fillStyle === FUTURE_FILL).map((c) => c.args)).toEqual([[750, 0, 250, RULER_H]]);
    const labelXs = named("fillText").map((c) => Number(c.args[1]));
    expect(labelXs.length).toBeGreaterThan(0);
    for (const x of labelXs) expect(x).toBeLessThan(750);
  });
});
