import { describe, expect, it } from "vitest";
import type { TopologyView } from "@karbird/web-engine";
import {
  areaPath,
  bucketRates,
  edgeEnds,
  edgeIndex,
  formatBytes,
  formatMs,
  formatPercent,
  formatRate,
  peakIndex,
  sizeNote,
  sizeState,
} from "./edge-traffic";

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
    expect(edgeEnds(topo, 1)).toEqual({ source: "svc_orders", target: "db" });
    expect(edgeIndex(topo, { source: "svc_orders", target: "db" })).toBe(1);
  });

  it("is direction-sensitive and null for unknown lines", () => {
    expect(edgeIndex(topo, { source: "db", target: "svc_orders" })).toBeNull();
    expect(edgeEnds(topo, 9)).toBeNull();
    expect(edgeEnds(null, 0)).toBeNull();
    expect(edgeIndex(topo, null)).toBeNull();
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
