import { describe, expect, it } from "vitest";
import type { TopologyView } from "@karbird/web-engine";
import {
  areaPath,
  applyLiveEdgeContribution,
  bucketRates,
  edgeEnds,
  edgeIndex,
  formatBytes,
  formatMs,
  formatPercent,
  formatRate,
  LiveEdgeAccumulator,
  peakIndex,
  sizeNote,
  sizeState,
  type EdgeTraffic,
} from "./edge-traffic";

function trafficFixture(): EdgeTraffic {
  return {
    source: "svc_a", target: "svc_b", relation: "calls", from_ns: "0", to_ns: "60000000000",
    buckets: 6, bucket_ns: "10000000000", resume_cursor_ns: "100", calls: [0, 0, 0, 0, 0, 0],
    sized_calls: [0, 0, 0, 0, 0, 0], request_sized_calls: [0, 0, 0, 0, 0, 0],
    response_sized_calls: [0, 0, 0, 0, 0, 0], request_bytes: [0, 0, 0, 0, 0, 0],
    response_bytes: [0, 0, 0, 0, 0, 0], total_calls: 0, total_sized_calls: 0,
    total_request_sized_calls: 0, total_response_sized_calls: 0, total_request_bytes: 0,
    total_response_bytes: 0, size_coverage: null, avg_request_bps: null, avg_response_bps: null,
    peak_request_bps: null, peak_request_at_ns: null, peak_response_bps: null, peak_response_at_ns: null,
  };
}

const spanSignal = (record: Record<string, unknown>, storageCursor: string) => ({ kind: "span", resume: storageCursor, storageCursor, record });

describe("live edge traffic", () => {
  it("resolves child-before-parent service calls once and ignores records covered by the baseline", () => {
    const accumulator = new LiveEdgeAccumulator({ source: "svc_a", target: "svc_b" }, "calls", true, 100n);
    const child = { trace_id: "t", span_id: "child", parent_span_id: "parent", service_id: "svc_b", kind: 2,
      start_ns: "20000000000", attributes: { "http.request.body.size": "120", "http.response.body.size": "300" } };
    const parent = { trace_id: "t", span_id: "parent", service_id: "svc_a", kind: 2, start_ns: "19000000000" };
    expect(accumulator.accept([spanSignal(child, "101")])).toEqual([]);
    expect(accumulator.accept([spanSignal(parent, "102")])).toEqual([
      { id: "t:child", timeNs: 20_000_000_000n, requestBytes: 120, responseBytes: 300, storageCursor: 101n },
    ]);
    expect(accumulator.accept([spanSignal(child, "103")])).toEqual([]);
    expect(accumulator.accept([spanSignal({ ...child, span_id: "old" }, "100")])).toEqual([]);
  });

  it("matches dependency spans by source and target entity", () => {
    const accumulator = new LiveEdgeAccumulator({ source: "svc_a", target: "db" }, "depends_on", true, 0n);
    expect(accumulator.accept([spanSignal({ trace_id: "t", span_id: "s", service_id: "svc_a", kind: 3,
      target_entity_id: "db", start_ns: "1000000000" }, "1")] )).toHaveLength(1);
    expect(accumulator.accept([spanSignal({ trace_id: "t", span_id: "other", service_id: "svc_a", kind: 3,
      target_entity_id: "cache", start_ns: "1000000000" }, "2")])).toEqual([]);
  });

  it("updates the matching bucket and keeps the cursor monotonic", () => {
    const updated = applyLiveEdgeContribution(trafficFixture(), {
      id: "t:s", timeNs: 25_000_000_000n, requestBytes: 1000, responseBytes: 500, storageCursor: 150n,
    }, 60_000_000_000n);
    expect(updated.calls).toEqual([0, 0, 1, 0, 0, 0]);
    expect(updated.request_bytes).toEqual([0, 0, 1000, 0, 0, 0]);
    expect(updated.total_calls).toBe(1);
    expect(updated.avg_request_bps).toBeCloseTo(1000 / 60);
    expect(updated.resume_cursor_ns).toBe("150");
    const older = applyLiveEdgeContribution(updated, {
      id: "t:s2", timeNs: 26_000_000_000n, requestBytes: null, responseBytes: null, storageCursor: 120n,
    }, 60_000_000_000n);
    expect(older.resume_cursor_ns).toBe("150");
  });

  it("slides the fixed bucket window when a live span advances beyond its end", () => {
    const updated = applyLiveEdgeContribution(trafficFixture(), {
      id: "t:late", timeNs: 65_000_000_000n, requestBytes: 10, responseBytes: null, storageCursor: 200n,
    }, 65_000_000_000n);
    expect(updated.from_ns).toBe("10000000000");
    expect(updated.to_ns).toBe("70000000000");
    expect(updated.calls).toEqual([0, 0, 0, 0, 0, 1]);
  });
});

describe("edgeEnds / edgeIndex", () => {
  const node = (index: number, id: string) => ({ index, id, name: id, kind: "service" as const, provenance: "observed" as const, x: 0, y: 0 });
  const edge = (index: number, source: number, target: number) => ({
    index,
    source,
    target,
    relation: "calls" as const,
    provenance: "observed" as const,
    reasons: [],
    points: [],
  });
  const topo: TopologyView = {
    revision: 1,
    nodes: [node(0, "svc_gw"), node(1, "svc_orders"), { ...node(2, "db"), kind: "database" }],
    edges: [edge(0, 0, 1), edge(1, 1, 2)],
    infra: null,
  };

  it("maps an engine edge index to the node IDs at its ends and back", () => {
    expect(edgeEnds(topo, 1)).toEqual({ source: "svc_orders", target: "db", relation:"calls" });
    expect(edgeIndex(topo, { source: "svc_orders", target: "db" })).toBe(1);
  });

  it("is direction-sensitive and null for unknown lines", () => {
    expect(edgeIndex(topo, { source: "db", target: "svc_orders" })).toBeNull();
    expect(edgeEnds(topo, 9)).toBeNull();
    expect(edgeEnds(null, 0)).toBeNull();
    expect(edgeIndex(topo, null)).toBeNull();
  });

  it("keeps a selected relation distinct when the same nodes have two kinds of links", () => {
    const graph: TopologyView = { ...topo, edges:[edge(0,0,1),{...edge(1,0,1),relation:"depends_on"}] };
    expect(edgeIndex(graph,{source:"svc_gw",target:"svc_orders",relation:"depends_on"})).toBe(1);
  });
});

describe("formatBytes / formatRate", () => {
  it("uses binary units with three significant digits", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(950)).toBe("950 B");
    expect(formatBytes(1024)).toBe("1 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(12.34 * 1024)).toBe("12.3 KB");
    expect(formatBytes(250 * 1024 * 1024)).toBe("250 MB");
    expect(formatBytes(3 * 1024 ** 3)).toBe("3 GB");
  });

  it("never shows garbage for bad input", () => {
    expect(formatBytes(-5)).toBe("0 B");
    expect(formatBytes(Number.NaN)).toBe("0 B");
  });

  it("marks unknown rates instead of showing zero", () => {
    expect(formatRate(null)).toBe("—");
    expect(formatRate(undefined)).toBe("—");
    expect(formatRate(0)).toBe("0 B/s");
    expect(formatRate(2048)).toBe("2 KB/s");
  });
});

describe("formatMs / formatPercent", () => {
  it("scales durations", () => {
    expect(formatMs(4.25)).toBe("4.3 ms");
    expect(formatMs(250.4)).toBe("250 ms");
    expect(formatMs(12_500)).toBe("12.5 s");
  });

  it("keeps small shares visible", () => {
    expect(formatPercent(0)).toBe("0%");
    expect(formatPercent(0.004)).toBe("0.4%");
    expect(formatPercent(0.857)).toBe("86%");
  });
});

describe("sizeState / sizeNote", () => {
  it("tells unknown sizes apart from no traffic and partial coverage", () => {
    expect(sizeState({ total_calls: 0, total_sized_calls: 0 })).toEqual({ kind: "no-calls" });
    expect(sizeState({ total_calls: 9, total_sized_calls: 0 })).toEqual({ kind: "unknown", calls: 9 });
    expect(sizeState({ total_calls: 14, total_sized_calls: 12 })).toEqual({ kind: "partial", sized: 12, calls: 14 });
    expect(sizeState({ total_calls: 5, total_sized_calls: 5 })).toEqual({ kind: "full", calls: 5 });
  });

  it("says plainly when services do not report sizes", () => {
    expect(sizeNote({ kind: "unknown", calls: 9 })).toMatch(/do not report request or response sizes/);
    expect(sizeNote({ kind: "partial", sized: 12, calls: 14 })).toBe("Size known for 12 of 14 calls (86%); rates count only those.");
    expect(
      sizeNote({ kind: "full", calls: 7 }, { calls: 7, requestSizedCalls: 0, responseSizedCalls: 7 }),
    ).toContain("Request size known for 0 of 7 calls; response size known for 7 of 7");
  });
});

describe("bucketRates / peakIndex / areaPath", () => {
  it("combines request and response bytes per second of bucket", () => {
    expect(bucketRates({ request_bytes: [1000, 0], response_bytes: [3000, 500], bucket_ns: "2000000000" })).toEqual([2000, 250]);
    expect(bucketRates({ request_bytes: [1], response_bytes: [1], bucket_ns: "0" })).toEqual([]);
  });

  it("finds the first peak and none for a flat zero series", () => {
    expect(peakIndex([1, 5, 5, 2])).toBe(1);
    expect(peakIndex([0, 0])).toBeNull();
    expect(peakIndex([])).toBeNull();
  });

  it("draws steps scaled so the peak touches the top", () => {
    expect(areaPath([0, 2, 1], 30, 10)).toBe("M0 10 L0 10 L10 10 L10 0 L20 0 L20 5 L30 5 L30 10 Z");
    expect(areaPath([0, 0], 20, 10)).toBe("M0 10 L0 10 L10 10 L10 10 L20 10 L20 10 Z");
    expect(areaPath([], 20, 10)).toBe("");
  });
});
