import { describe, expect, it } from "vitest";
import {
  MIN_VIEW_NS,
  MAX_VIEW_NS,
  MAX_DETAIL_RANGE_NS,
  clampRangeToNow,
  clampView,
  visibleRangeSegment,
  liveSummaryWindow,
  expandRange,
  timelineBounds,
  mergeRanges,
  cachedSummaryRanges,
  trimRangesBefore,
  timelineEvictionFloor,
  pointLoadingRange,
  detailChunkFor,
  defaultLiveView,
  expandSummaryWindow,
  needsTimelineSummary,
  rangeCoveredBy,
  uncoveredRanges,
  follow,
  followLive,
  formatSpan,
  formatTick,
  isNearPlayhead,
  panBy,
  tickStep,
  ticks,
  timeAt,
  xOf,
  zoomAt,
  zoomCentered,
} from "./scale";

const MS = 1_000_000n;
const SEC = 1_000_000_000n;
const T0 = 1_790_000_000n * SEC;
const bounds = { from: T0, to: T0 + 900n * SEC };

describe("playhead hit area", () => {
  it("detects the playhead within eight pixels", () => {
    const view = { from: 0n, to: 1_000_000_000n };
    expect(isNearPlayhead(496, 500_000_000n, view, 1000)).toBe(true);
    expect(isNearPlayhead(509, 500_000_000n, view, 1000)).toBe(false);
  });
});

describe("followLive", () => {
  const v = { from: T0 + 100n * SEC, to: T0 + 200n * SEC };

  it("leaves the view alone while the playhead is inside it", () => {
    expect(followLive(v, T0 + 150n * SEC, bounds)).toBe(v);
    expect(followLive(v, T0 + 190n * SEC, bounds)).toBe(v);
  });

  it("moves the window so the live playhead sits on the right edge", () => {
    const moved = followLive(v, T0 + 201n * SEC, bounds);
    expect(moved.to - moved.from).toBe(100n * SEC);
    expect(xOf(T0 + 201n * SEC, moved, 1000)).toBeCloseTo(1000, 6);
    expect(followLive(moved, T0 + 201n * SEC, bounds)).toBe(moved);
  });

  it("jumps back to now when the view was scrolled into the past", () => {
    const back = followLive(v, T0 + 500n * SEC, bounds);
    expect(xOf(T0 + 500n * SEC, back, 1000)).toBeCloseTo(1000, 6);
  });

  it("stays inside the shown range", () => {
    const end = followLive(v, bounds.to, bounds);
    expect(end.to).toBe(bounds.to);
  });
});

describe("zoomCentered", () => {
  it("centres the window on the cursor, within the loaded range", () => {
    const z = zoomCentered(bounds, T0 + 300n * SEC, 0.1, bounds);
    expect(z.to - z.from).toBe((bounds.to - bounds.from) / 10n);
    expect(z.from + (z.to - z.from) / 2n).toBe(T0 + 300n * SEC);
    const edge = zoomCentered(bounds, bounds.to, 0.1, bounds);
    expect(edge.to).toBe(bounds.to);
  });

  it("allows one month of history without zooming out farther", () => {
    const wide = { from: T0 - 90n * 86_400n * SEC, to: T0 };
    let view = { from: T0 - 7n * 86_400n * SEC, to: T0 };
    for (let i = 0; i < 20; i++) view = zoomCentered(view, T0, 2, wide);
    expect(view.to - view.from).toBe(MAX_VIEW_NS);
    expect(formatSpan(MAX_VIEW_NS)).toBe("30 days");
    expect(tickStep(view, 1200)).toBe(3n * 86_400n * SEC);
  });
});

describe("detailChunkFor", () => {
  it("requests a bounded detail window centred on the visible range", () => {
    const view = { from: T0 - 10n * 86_400n * SEC, to: T0 };
    const chunk = detailChunkFor(view);
    expect(chunk.to - chunk.from).toBe(MAX_DETAIL_RANGE_NS);
    expect(chunk.from + (chunk.to - chunk.from) / 2n).toBe(T0 - 5n * 86_400n * SEC);
  });

  it("does not request detail data beyond the supplied live-time ceiling", () => {
    const now = T0 + 123n;
    const chunk = detailChunkFor({ from: now - 1n, to: now }, now);
    expect(chunk.to).toBe(now);
    expect(chunk.from).toBe(now - MAX_DETAIL_RANGE_NS);
  });
});

describe("defaultLiveView", () => {
  it("opens on the latest hour and ends exactly at now", () => {
    const now = T0 + 123n;
    expect(defaultLiveView(now)).toEqual({ from: now - 3_600n * SEC, to: now });
  });
});

describe("needsTimelineSummary", () => {
  it("uses the summary for wider views and raw rows at detail zoom", () => {
    expect(needsTimelineSummary({ from: T0 - 60n * SEC, to: T0 })).toBe(false);
    expect(needsTimelineSummary({ from: T0 - 3_600n * SEC, to: T0 })).toBe(true);
  });
});

describe("expandRange", () => {
  it("keeps a loaded summary start stable while extending its right edge", () => {
    expect(expandRange(
      { from: T0 - 3_600n * SEC, to: T0 + 10n * SEC },
      { from: T0 - 3_000n * SEC, to: T0 + 20n * SEC },
    )).toEqual({ from: T0 - 3_600n * SEC, to: T0 + 20n * SEC });
  });

  it("extends coverage when the user pans into earlier history", () => {
    expect(expandRange(
      { from: T0 - 3_000n * SEC, to: T0 + 20n * SEC },
      { from: T0 - 3_600n * SEC, to: T0 + 10n * SEC },
    )).toEqual({ from: T0 - 3_600n * SEC, to: T0 + 20n * SEC });
  });
});

describe("expandSummaryWindow", () => {
  it("pages into older history without requesting beyond the 30 day cap", () => {
    const current = { from: T0 - MAX_VIEW_NS, to: T0 };
    const shift = 3n * 86_400n * SEC;
    const older = { from: current.from - shift, to: current.to - shift };
    expect(expandSummaryWindow(current, older)).toEqual(older);
  });

  it("keeps overlapping summary coverage when the union fits the cap", () => {
    const current = { from: T0 - 10n * 86_400n * SEC, to: T0 };
    const shift = 2n * 86_400n * SEC;
    const next = { from: current.from - shift, to: current.to - shift };
    expect(expandSummaryWindow(current, next)).toEqual({ from: next.from, to: current.to });
  });
});

describe("timelineBounds", () => {
  it("allows older history until the server confirms the oldest available timestamp", () => {
    const latest = T0 + 60n * SEC;
    expect(timelineBounds({ status: "unknown" }, latest)).toEqual({ from: 0n, to: latest });
    expect(timelineBounds({ status: "known", oldestNs: T0 }, latest)).toEqual({ from: T0, to: latest });
  });

  it("stops navigation when the server confirms there is no timeline data", () => {
    expect(timelineBounds({ status: "known", oldestNs: null }, T0)).toEqual({ from: T0 - MAX_VIEW_NS, to: T0 });
  });

  it("clamps a remembered view when the server confirms an older boundary", () => {
    const oldest = T0;
    const allowed = timelineBounds({ status: "known", oldestNs: oldest }, T0 + 3n * MAX_VIEW_NS);
    expect(clampView({ from: oldest - 10n * SEC, to: oldest + 10n * SEC }, allowed).from).toBe(oldest);
  });
});

describe("clampRangeToNow", () => {
  it("clamps the requested end to now and rejects future-only windows", () => {
    const now = T0 + 100n;
    expect(clampRangeToNow({ from: now - 10n, to: now + 50n }, now)).toEqual({ from: now - 10n, to: now });
    expect(clampRangeToNow({ from: now + 1n, to: now + 20n }, now)).toBeNull();
  });
});

describe("visibleRangeSegment", () => {
  const view = { from: 100n, to: 200n };

  it("maps the requested time segment to the visible timeline width", () => {
    expect(visibleRangeSegment({ from: 125n, to: 175n }, view, 400)).toEqual({ left: 100, width: 200 });
  });

  it("clips the requested segment at the visible timeline boundaries", () => {
    expect(visibleRangeSegment({ from: 50n, to: 125n }, view, 400)).toEqual({ left: 0, width: 100 });
    expect(visibleRangeSegment({ from: 175n, to: 250n }, view, 400)).toEqual({ left: 300, width: 100 });
  });

  it("returns null when the requested range does not overlap the view", () => {
    expect(visibleRangeSegment({ from: 0n, to: 100n }, view, 400)).toBeNull();
    expect(visibleRangeSegment({ from: 200n, to: 300n }, view, 400)).toBeNull();
    expect(visibleRangeSegment({ from: 125n, to: 175n }, view, 0)).toBeNull();
  });
});

describe("timeline loading coverage", () => {
  it("merges overlapping and adjacent loaded intervals", () => {
    expect(mergeRanges([{ from: 10n, to: 20n }, { from: 30n, to: 40n }], { from: 20n, to: 32n }))
      .toEqual([{ from: 10n, to: 40n }]);
  });

  it("trims evicted coverage at the retention edge", () => {
    expect(trimRangesBefore([{ from: 0n, to: 10n }, { from: 15n, to: 30n }], 20n))
      .toEqual([{ from: 20n, to: 30n }]);
  });

  it("retains previously loaded history while paused and uses the rolling window in live mode", () => {
    const loaded = [{ from: 10n, to: 20n }, { from: 90n, to: 100n }];
    expect(timelineEvictionFloor("paused", 80n, 95n, loaded)).toBe(10n);
    expect(timelineEvictionFloor("historical", 5n, 95n, loaded)).toBe(5n);
    expect(timelineEvictionFloor("live", 80n, 95n, loaded)).toBe(95n);
  });

  it("marks only a narrow interval around a selected point", () => {
    expect(pointLoadingRange(50n, { from: 0n, to: 100n }, 5n)).toEqual({ from: 45n, to: 55n });
    expect(pointLoadingRange(2n, { from: 0n, to: 100n }, 5n)).toEqual({ from: 0n, to: 7n });
  });
});

describe("live summary range", () => {
  it("keeps the live summary request stable within its 10-second bucket", () => {
    const first = { from: 101n * SEC, to: 4_601n * SEC };
    const shifted = { from: first.from + SEC, to: first.to + SEC };
    expect(liveSummaryWindow(shifted)).toEqual(liveSummaryWindow(first));
    expect(liveSummaryWindow({ from: first.from + 10n * SEC, to: first.to + 10n * SEC }))
      .not.toEqual(liveSummaryWindow(first));
  });

  it("extends the loaded summary horizon while its viewed start stays fixed", () => {
    const from = 101n * SEC;
    const first = liveSummaryWindow({ from, to: 4_601n * SEC });
    const later = liveSummaryWindow({ from, to: 4_611n * SEC });
    expect(later.from).toBe(first.from);
    expect(later.to).toBeGreaterThan(first.to);
  });

});

describe("rangeCoveredBy", () => {
  it("recognizes adjacent loaded intervals but keeps gaps unloaded", () => {
    const loaded = [{ from: 0n, to: 10n }, { from: 10n, to: 20n }, { from: 30n, to: 40n }];
    expect(rangeCoveredBy(loaded, { from: 5n, to: 18n })).toBe(true);
    expect(rangeCoveredBy(loaded, { from: 5n, to: 35n })).toBe(false);
  });
});

describe("uncoveredRanges", () => {
  it("returns only missing intervals within the requested range", () => {
    const loaded = [{ from: 10n, to: 20n }, { from: 30n, to: 40n }];
    expect(uncoveredRanges(loaded, { from: 0n, to: 50n })).toEqual([
      { from: 0n, to: 10n },
      { from: 20n, to: 30n },
      { from: 40n, to: 50n },
    ]);
  });

  it("returns no intervals when the request is already loaded", () => {
    expect(uncoveredRanges([{ from: 0n, to: 100n }], { from: 20n, to: 80n })).toEqual([]);
  });

  it("keeps previously visited disjoint windows covered when the cursor returns to either one", () => {
    const coverage = mergeRanges([{ from: 0n, to: 20n }], { from: 80n, to: 100n });
    expect(uncoveredRanges(coverage, { from: 82n, to: 90n })).toEqual([]);
    expect(uncoveredRanges(coverage, { from: 10n, to: 18n })).toEqual([]);
    expect(uncoveredRanges(coverage, { from: 40n, to: 50n })).toEqual([{ from: 40n, to: 50n }]);
  });
});

describe("cachedSummaryRanges", () => {
  it("retains validated disjoint windows from the query cache", () => {
    expect(cachedSummaryRanges([
      { from_ns: "0", to_ns: "20" },
      { from_ns: "80", to_ns: "100" },
      { from_ns: "invalid", to_ns: "200" },
      null,
    ])).toEqual([{ from: 0n, to: 20n }, { from: 80n, to: 100n }]);
  });
});

describe("zoomAt", () => {
  it("keeps the anchor at the same screen position", () => {
    const v = { from: T0 + 100n * SEC, to: T0 + 200n * SEC };
    const anchor = T0 + 125n * SEC;
    const z = zoomAt(v, anchor, 0.5, bounds);
    expect(z.to - z.from).toBe(50n * SEC);
    expect(xOf(anchor, z, 1000)).toBeCloseTo(xOf(anchor, v, 1000), 6);
  });

  it("stops at one millisecond and at the loaded range", () => {
    let v = { ...bounds };
    for (let i = 0; i < 100; i++) v = zoomAt(v, T0 + 450n * SEC, 0.25, bounds);
    expect(v.to - v.from).toBe(MIN_VIEW_NS);
    for (let i = 0; i < 100; i++) v = zoomAt(v, T0, 4, bounds);
    expect(v).toEqual(bounds);
  });
});

describe("panBy and follow", () => {
  it("clamps to the loaded range without changing width", () => {
    const v = { from: T0 + 10n * SEC, to: T0 + 20n * SEC };
    expect(panBy(v, -60n * SEC, bounds)).toEqual({ from: T0, to: T0 + 10n * SEC });
    expect(panBy(v, 10_000n * SEC, bounds)).toEqual({ from: bounds.to - 10n * SEC, to: bounds.to });
  });

  it("pages so the playhead sits a quarter into the window", () => {
    const v = { from: T0, to: T0 + 8n * MS };
    expect(follow(v, T0 + 4n * MS, bounds)).toBe(v);
    const f = follow(v, T0 + 20n * MS, bounds);
    expect(f).toEqual({ from: T0 + 18n * MS, to: T0 + 26n * MS });
  });

  it("settles when the playhead is outside the loaded range", () => {
    const v = { from: T0 + 300n * SEC, to: T0 + 600n * SEC };
    for (const t of [T0 - 3n * MS, bounds.to + 5n * SEC]) {
      const live = (view: typeof v, time: bigint, b: typeof bounds) => followLive(view, time, b);
      for (const move of [follow, live]) {
        const once = move(v, t, bounds);
        expect(move(once, t, bounds)).toBe(once);
        expect(once.from >= bounds.from && once.to <= bounds.to).toBe(true);
      }
    }
  });
});

describe("ticks", () => {
  it("chooses finer steps as the view narrows, down to microseconds", () => {
    expect(tickStep({ from: T0, to: T0 + 900n * SEC }, 1000)).toBe(2n * 60n * SEC);
    expect(tickStep({ from: T0, to: T0 + 10n * SEC }, 1000)).toBe(1n * SEC);
    expect(tickStep({ from: T0, to: T0 + MS }, 1000)).toBe(100_000n);
  });

  it("aligns ticks to the step and stays inside the view", () => {
    const v = { from: T0 + 1_234_567n, to: T0 + 3n * MS };
    const ts = ticks(v, MS);
    expect(ts.map((t) => (t - T0) / MS)).toEqual([2n, 3n]);
  });

  it("formats labels with precision matching the step", () => {
    const t = T0 + 12_345_678n;
    expect(formatTick(t, SEC)).toMatch(/^\d\d:\d\d:\d\d$/);
    expect(formatTick(t, MS)).toMatch(/^\d\d:\d\d\.012$/);
    expect(formatTick(t, 100_000n)).toMatch(/^\d\d\.012 345$/);
    expect(formatSpan(2_500_000n)).toBe("2.5 ms");
    expect(formatSpan(40n * SEC)).toBe("40 s");
  });

  it("maps pixels back to time", () => {
    const v = { from: T0, to: T0 + 10n * MS };
    expect(timeAt(250, v, 1000)).toBe(T0 + 2_500_000n);
  });
});
