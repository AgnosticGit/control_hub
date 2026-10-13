import { describe, expect, it } from "vitest";
import type { Schemas } from "@/lib/api";
import { compareServices, comparisonWindow, comparisonWarnings } from "./change-replay";

type Service = Schemas["ServiceDto"];
const service = (id: string, requests: number, errors: number, p95_ms: number | null): Service => ({
  service_id: id, name: id, requests, errors, p95_ms, last_seen_ns: null,
  instances: 1, version: null, logs: 0, error_logs: 0, signals: ["traces"],
});
const completeness = (watermark_ns: string | null): Schemas["Completeness"] => ({
  watermark_ns, processing_caught_up: true, unresolved_processing_errors: 0,
  truncated: false, sampling: "unknown", notes: [],
});

describe("release comparison windows", () => {
  it("uses equal 15-minute windows around a completed release", () => {
    const at = 1_800_000_000_000_000_000n;
    expect(comparisonWindow(at, at + 30n * 60n * 1_000_000_000n)).toEqual({
      before: { from: at - 15n * 60n * 1_000_000_000n, to: at },
      after: { from: at, to: at + 15n * 60n * 1_000_000_000n },
      provisional: false,
    });
  });

  it("shortens both windows equally while the post-release window is still open", () => {
    const at = 1_800_000_000_000_000_000n;
    expect(comparisonWindow(at, at + 3n * 60n * 1_000_000_000n)).toEqual({
      before: { from: at - 3n * 60n * 1_000_000_000n, to: at },
      after: { from: at, to: at + 3n * 60n * 1_000_000_000n },
      provisional: true,
    });
    expect(comparisonWindow(at, at)).toBeNull();
  });
});

describe("release comparison evidence", () => {
  it("keeps the released service first and compares rates only when both windows have requests", () => {
    const rows = compareServices("orders", [service("gateway", 20, 1, 50), service("orders", 10, 1, 100)], [
      service("orders", 10, 4, 250), service("catalog", 3, 0, 30), service("gateway", 0, 0, null),
    ]);
    expect(rows.map((row) => row.serviceId)).toEqual(["orders", "gateway", "catalog"]);
    expect(rows[0]?.errorRateBefore).toBe(10);
    expect(rows[0]?.errorRateAfter).toBe(40);
    expect(rows[0]?.p95DeltaMs).toBe(150);
    expect(rows[1]?.errorRateAfter).toBeNull();
    expect(rows[2]?.errorRateBefore).toBeNull();
  });

  it("marks an unfinished or lagging comparison without claiming complete evidence", () => {
    const at = 1_800_000_000_000_000_000n;
    const window = comparisonWindow(at, at + 20n * 60n * 1_000_000_000n);
    expect(window).not.toBeNull();
    if (!window) return;
    expect(comparisonWarnings(completeness((window.after.to - 1n).toString()), completeness((window.after.to - 1n).toString()), window))
      .toContain("Processing has not reached the end of the comparison window.");
    expect(comparisonWarnings(completeness(window.after.to.toString()), completeness(window.after.to.toString()), window))
      .not.toContain("Processing has not reached the end of the comparison window.");
  });
});
