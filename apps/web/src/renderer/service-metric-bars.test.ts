import { describe, expect, it } from "vitest";
import { metricBar, METRIC_BAR_SEGMENTS } from "./service-metric-bars";

describe("segmented service metric bars", () => {
  it("fills segments proportionally and changes color at the agreed thresholds", () => {
    expect(metricBar(0.5)).toEqual({ filled: 6, color: 0x22c55e, segments: METRIC_BAR_SEGMENTS });
    expect(metricBar(0.7)).toEqual({ filled: 8, color: 0xf5b942, segments: METRIC_BAR_SEGMENTS });
    expect(metricBar(0.9)).toEqual({ filled: 11, color: 0xef4444, segments: METRIC_BAR_SEGMENTS });
  });

  it("clamps over-capacity utilization and keeps missing data unfilled", () => {
    expect(metricBar(1.4)).toMatchObject({ filled: METRIC_BAR_SEGMENTS, color: 0xef4444 });
    expect(metricBar(null)).toMatchObject({ filled: 0, color: null });
    expect(metricBar(Number.NaN)).toMatchObject({ filled: 0, color: null });
  });
});
