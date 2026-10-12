import { describe, expect, it } from "vitest";
import { missingTimelineRanges, oldestTimelineRange, TIMELINE_CACHE_LIMIT_BYTES } from "./timeline-cache";

describe("timeline cache range policy", () => {
  it("requests only the gaps between cached half-open ranges", () => {
    expect(missingTimelineRanges(0n, 100n, [{ fromNs: "10", toNs: "30" }, { fromNs: "50", toNs: "80" }]))
      .toEqual([{ fromNs: "0", toNs: "10" }, { fromNs: "30", toNs: "50" }, { fromNs: "80", toNs: "100" }]);
  });

  it("merges overlaps and clips coverage to the requested interval", () => {
    expect(missingTimelineRanges(5n, 25n, [{ fromNs: "0", toNs: "10" }, { fromNs: "8", toNs: "30" }])).toEqual([]);
  });

  it("chooses the least recently used complete range first", () => {
    expect(oldestTimelineRange([
      { id: "new", touchedAt: 20 }, { id: "old", touchedAt: 10 }, { id: "mid", touchedAt: 15 },
    ])).toBe("old");
    expect(oldestTimelineRange([])).toBeNull();
  });

  it("uses the exact one-gibibyte storage budget", () => {
    expect(TIMELINE_CACHE_LIMIT_BYTES).toBe(1_073_741_824);
  });
});
