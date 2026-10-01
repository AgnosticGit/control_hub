import { describe, expect, it } from "vitest";
import { colorAt } from "@/renderer/service-colors";
import {
  BUCKET,
  EDGES,
  FAULT_FAILING,
  FAULT_START,
  MARKERS,
  NODES,
  VIEW,
  WINDOW,
  clampT,
  dotsAt,
  edgeCurve,
  healthAt,
  pointOnCurve,
  relativeLabel,
  requestsAt,
  serviceColor,
  stateAt,
  timelineBuckets,
} from "./demo-model";

describe("landing demo model", () => {
  it("is deterministic: the same time always gives the same picture", () => {
    for (const t of [0, 123.4, FAULT_START, 777.7, WINDOW]) {
      expect(stateAt(t)).toEqual(stateAt(t));
    }
    expect(timelineBuckets()).toEqual(timelineBuckets());
  });

  it("shows orders healthy before the fault, degraded, then failing", () => {
    expect(healthAt("orders", FAULT_START - 1)).toBe("ok");
    expect(healthAt("orders", FAULT_START)).toBe("degraded");
    expect(healthAt("orders", FAULT_FAILING)).toBe("failing");
    expect(healthAt("orders", WINDOW)).toBe("failing");
    expect(healthAt("auth", WINDOW)).toBe("ok");
  });

  it("raises latency and error rate only while the fault is active", () => {
    const before = stateAt(FAULT_START - 10);
    const after = stateAt(WINDOW);
    expect(before.errorRate.orders).toBe(0);
    expect(after.errorRate.orders).toBeGreaterThan(0.5);
    expect(after.p95ms.orders).toBeGreaterThan(before.p95ms.orders ?? 0);
  });

  it("clamps time to the window and treats non-finite input as now", () => {
    expect(clampT(-5)).toBe(0);
    expect(clampT(WINDOW + 100)).toBe(WINDOW);
    expect(clampT(Number.NaN)).toBe(WINDOW);
    expect(stateAt(-100).t).toBe(0);
  });

  it("keeps every request dot on a known, observed edge and within it", () => {
    for (let t = 0; t <= WINDOW; t += 7.3) {
      for (const dot of dotsAt(t)) {
        expect(EDGES[dot.edge]?.kind).toBe("observed");
        expect(dot.at).toBeGreaterThanOrEqual(0);
        expect(dot.at).toBeLessThanOrEqual(1);
      }
    }
  });

  it("marks requests as errors only where a failing service is involved", () => {
    expect(dotsAt(100).some((x) => x.error)).toBe(false);
    const late = Array.from({ length: 40 }, (_, i) => dotsAt(WINDOW - i * 0.5)).flat();
    for (const dot of late.filter((x) => x.error)) {
      const edge = EDGES[dot.edge];
      expect([edge?.from, edge?.to]).toContain("orders");
    }
    expect(late.some((x) => x.error)).toBe(true);
  });

  it("labels time relative to now", () => {
    expect(relativeLabel(WINDOW)).toBe("now");
    expect(relativeLabel(WINDOW - 65)).toBe("−1:05");
    expect(relativeLabel(0)).toBe("−15:00");
  });

  it("connects every edge to known nodes and has one inferred edge with a reason", () => {
    const ids = new Set(NODES.map((n) => n.id));
    for (const e of EDGES) expect(ids.has(e.from) && ids.has(e.to)).toBe(true);
    const inferred = EDGES.filter((e) => e.kind === "inferred");
    expect(inferred).toHaveLength(1);
    expect(inferred[0]?.reason).toMatch(/Inferred/);
  });

  it("places markers inside the window", () => {
    for (const m of MARKERS) {
      expect(m.t).toBeGreaterThan(0);
      expect(m.t).toBeLessThan(WINDOW);
    }
  });

  it("colours services the way the workspace does and leaves external systems neutral", () => {
    const services = NODES.filter((n) => !n.external)
      .map((n) => n.id)
      .sort();
    services.forEach((id, i) => expect(serviceColor(id)).toBe(colorAt(i)));
    expect(serviceColor("clients")).toBeNull();
    expect(serviceColor("provider")).toBeNull();
    expect(new Set(services.map(serviceColor)).size).toBe(services.length);
  });

  for (const o of ["wide", "narrow"] as const) {
    it(`keeps cards apart and inside the view in the ${o} layout`, () => {
      const { w, h, card } = VIEW[o];
      for (const n of NODES) {
        const [x, y] = n[o];
        expect(x - card.w / 2).toBeGreaterThanOrEqual(0);
        expect(x + card.w / 2).toBeLessThanOrEqual(w);
        expect(y - card.h / 2).toBeGreaterThanOrEqual(0);
        expect(y + card.h / 2).toBeLessThanOrEqual(h);
      }
      for (const a of NODES) {
        for (const b of NODES) {
          if (a === b) continue;
          const apart = Math.abs(a[o][0] - b[o][0]) >= card.w + 8 || Math.abs(a[o][1] - b[o][1]) >= card.h + 8;
          expect(apart, `${a.id} overlaps ${b.id}`).toBe(true);
        }
      }
    });

    it(`starts every ${o} edge at its source card and ends it at its target card`, () => {
      const { card } = VIEW[o];
      for (const e of EDGES) {
        const c = edgeCurve(e, o);
        const a = NODES.find((n) => n.id === e.from);
        const b = NODES.find((n) => n.id === e.to);
        expect(c && a && b).toBeTruthy();
        if (!c || !a || !b) continue;
        const [sx, sy] = pointOnCurve(c, 0);
        const [tx, ty] = pointOnCurve(c, 1);
        expect(Math.abs(sx - a[o][0])).toBeLessThanOrEqual(card.w / 2);
        expect(Math.abs(sy - a[o][1])).toBeLessThanOrEqual(card.h / 2);
        expect(Math.abs(tx - b[o][0])).toBeLessThanOrEqual(card.w / 2 + 6);
        expect(Math.abs(ty - b[o][1])).toBeLessThanOrEqual(card.h / 2 + 6);
      }
    });
  }

  it("clamps the curve parameter", () => {
    const edge = EDGES[0];
    const c = edge ? edgeCurve(edge, "wide") : null;
    expect(c).not.toBeNull();
    if (!c) return;
    expect(pointOnCurve(c, -1)).toEqual(pointOnCurve(c, 0));
    expect(pointOnCurve(c, 2)).toEqual(pointOnCurve(c, 1));
  });

  it("drops traffic through a failing service and keeps request counts positive elsewhere", () => {
    expect(requestsAt("orders", WINDOW)).toBeLessThan(requestsAt("orders", FAULT_START - 10));
    for (const n of NODES) {
      if (n.id === "notifications") continue;
      expect(requestsAt(n.id, 0)).toBeGreaterThan(0);
    }
    expect(requestsAt("notifications", WINDOW)).toBe(0);
  });

  it("fills the timeline: no errors before the fault, errors and higher latency after, events at markers", () => {
    const buckets = timelineBuckets();
    expect(buckets).toHaveLength(WINDOW / BUCKET);
    const before = buckets.filter((b) => b.t + BUCKET <= FAULT_START);
    const after = buckets.filter((b) => b.t >= FAULT_FAILING);
    expect(before.every((b) => b.errors === 0)).toBe(true);
    expect(after.every((b) => b.errors > 0)).toBe(true);
    expect(Math.min(...after.map((b) => b.latency))).toBeGreaterThan(Math.max(...before.map((b) => b.latency)));
    for (const m of MARKERS) {
      expect(buckets[Math.floor(m.t / BUCKET)]?.events).toBeGreaterThan(0);
    }
  });
});
