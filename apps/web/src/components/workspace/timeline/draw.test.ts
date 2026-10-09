import type { TimelineTracks } from "@karbird/web-engine";
import { describe, expect, it } from "vitest";
import { RULER_H, actionColor, bucketGeometry, draw, drawRuler, rowTop, totalHeight, withAlpha, type DrawInput, type Row } from "./draw";
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
    prev_item_ns: null,
    next_item_ns: null,
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
    expect(actionColor("succeeded")).toBe("#18d889");
    expect(actionColor("failed")).toBe("#ef4444");
    expect(actionColor("verification_failed")).toBe("#ef4444");
    expect(actionColor("running")).toBe("#f59e0b");
    expect(actionColor("approved")).toBe("#f59e0b");
    expect(actionColor("pending_approval")).toBe("#91a5bd");
    expect(actionColor("")).toBe("#91a5bd");
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
    const alt = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "rgba(18, 35, 56, 0.58)");
    expect(alt.map((c) => c.args)).toEqual([[0, 10, WIDTH, 15]]);
  });

  it("draws bars against a fixed count scale", () => {
    const { ctx, calls } = fakeCtx();
    draw(ctx, input({ rows: [row("r", "requests", 25)], tracks: tracks({ requests: [0, 5, 10] }) }));
    const bars = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "#3b82f6");
    expect(bars).toHaveLength(2);
    const heights = bars.map((b) => b.args[3]);
    expect(heights[0]).toBeCloseTo(Math.sqrt(0.05) * 20);
    expect(heights[1]).toBeCloseTo(Math.sqrt(0.1) * 20);
  });

  it("keeps a bucket's height stable when a later live tick adds a higher bucket", () => {
    const heightForFirstBucket = (requests: number[]) => {
      const { ctx, calls } = fakeCtx();
      draw(ctx, input({ rows: [row("r", "requests", 25)], tracks: tracks({ requests }) }));
      return calls.find((c) => c.name === "fillRect" && c.fillStyle === "#3b82f6")?.args[3];
    };

    expect(heightForFirstBucket([5, 10])).toBe(heightForFirstBucket([5, 40]));
  });

  it("keeps a service bucket's intensity stable when another bucket changes", () => {
    const firstBucketColor = (counts: number[]) => {
      const { ctx, calls } = fakeCtx();
      draw(ctx, input({ rows: [row("s", "service", 20, { track: 0, color: "#a78bfa" })], tracks: tracks({ services: [{ node: 0, name: "orders", counts, spans_in_view: 20, clipped: false }] }) }));
      return calls.find((c) => c.name === "fillRect" && String(c.fillStyle).startsWith("rgba(167, 139, 250,"))?.fillStyle;
    };

    expect(firstBucketColor([5, 10])).toBe(firstBucketColor([5, 40]));
  });

  it("keeps a latency sample's height stable when another bucket changes", () => {
    const firstPointY = (latency_p95_ms: Array<number | null>) => {
      const { ctx, named } = fakeCtx();
      draw(ctx, input({ rows: [row("l", "latency", 30)], tracks: tracks({ latency_p95_ms }) }));
      return named("moveTo")[0]?.args[1];
    };

    expect(firstPointY([100, 200])).toBe(firstPointY([100, 800]));
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

  it("draws machine CPU on an absolute scale, hot buckets in red, and memory as a broken line", () => {
    const { ctx, calls, named } = fakeCtx();
    const machines = {
      from_ns: "0",
      to_ns: "1000000000",
      buckets: 4,
      bucket_ns: "250000000",
      hosts: [{ id: "h", name: "h", cpu: [0.5, null, 0.95, 0.1], memory: [0.4, 0.5, null, 0.6] }],
      events: [],
      events_total: 0,
      truncated: false,
    };
    draw(ctx, input({ rows: [row("m", "machine", 26, { host: 0 }), row("x", "machine", 26, { host: 3 })], machines, cursor: -10n }));
    const cool = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "rgba(34, 211, 238, 0.5)");
    const hot = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "rgba(239, 68, 68, 0.75)");
    // 26 px row minus 6 px padding: 50 % is 10 px tall, not scaled to the busiest bucket.
    expect(cool.map((c) => [c.args[0], c.args[3]])).toEqual([
      [0, 10],
      [750, 2],
    ]);
    expect(hot.map((c) => c.args[0])).toEqual([500]);
    expect(named("moveTo").map((c) => c.args[0])).toEqual([125, 875]);
    expect(named("lineTo").map((c) => c.args[0])).toEqual([375]);
  });

  it("draws machine rows as empty without machine data", () => {
    const { ctx, named } = fakeCtx();
    draw(ctx, input({ rows: [row("hd", "heading", 22), row("m", "machine", 26, { host: 0 })], cursor: -10n }));
    expect(named("moveTo")).toHaveLength(0);
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
    expect(fills.map((f) => f.fillStyle)).toContain("#18d889");
    expect(fills.map((f) => f.fillStyle)).not.toContain("#ef4444");
    expect(named("fillText").map((c) => c.args[0])).toEqual(["Restart · orders"]);
  });

  it("shades the time outside the replay range", () => {
    const { ctx, calls } = fakeCtx();
    const rows = [row("g", "group", 40)];
    draw(ctx, input({ rows, range: { from: 250_000_000n, to: 750_000_000n }, cursor: -10n }));
    const shade = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "rgba(4, 12, 21, 0.6)");
    expect(shade.map((c) => c.args)).toEqual([
      [0, 0, 250, 40],
      [750, 0, 250, 40],
    ]);
    const edges = calls.filter((c) => c.name === "fillRect" && c.fillStyle === "rgba(25, 212, 238, 0.8)");
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
    expect(inView.calls.some((c) => c.name === "fillRect" && c.fillStyle === "#19d4ee")).toBe(true);
    const outside = fakeCtx();
    draw(outside.ctx, input({ rows: [row("g", "group", 40)], cursor: 5_000_000_000n }));
    expect(outside.calls.some((c) => c.name === "fillRect" && c.fillStyle === "#19d4ee")).toBe(false);
  });

  it("does not draw a separate future zone", () => {
    const { ctx, calls } = fakeCtx();
    draw(ctx, input({ rows: [row("g", "group", 40)], cursor: 900_000_000n }));
    expect(calls.some((c) => c.fillStyle === "rgba(100, 116, 139, 0.13)" || c.fillStyle === "rgba(148, 163, 184, 0.55)")).toBe(false);
    expect(calls.some((c) => c.name === "fillRect" && c.fillStyle === "#19d4ee")).toBe(true);
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

  it("keeps normal tick labels across the ruler", () => {
    const { ctx, named } = fakeCtx();
    drawRuler(ctx, { width: WIDTH, view: VIEW, cursor: 0n, markA: null, hoverX: null, range: null });
    expect(named("fillText").length).toBeGreaterThan(2);
  });
});
