import { describe, expect, it } from "vitest";
import type { Schemas } from "@/lib/api";
import { hasMetricOverviewData, metricOverviewTiles } from "./metric-overview";

const empty: Schemas["MetricOverviewResponse"] = {
  at_ns: "1000000000", window_seconds: 300, http_samples: 0, http_p95_ms: null,
  process_rss_bytes: null, process_cpu_pct_of_core: null, postgres_backends: null,
  postgres_commits_per_second: null, postgres_rollbacks_per_second: null, postgres_deadlocks_per_second: null,
};

describe("metric overview", () => {
  it("shows missing coverage as missing instead of zero latency or CPU", () => {
    expect(hasMetricOverviewData(empty, false)).toBe(false);
    expect(metricOverviewTiles(empty, false).map((tile) => tile.value)).toEqual(["—", "0", "—", "—"]);
  });

  it("labels histogram p95 as an upper bound and formats process and dependency measurements", () => {
    const data = { ...empty, http_samples: 10, http_p95_ms: 500, process_rss_bytes: 1_572_864,
      process_cpu_pct_of_core: 125.5, postgres_backends: 3, postgres_commits_per_second: 1.25 };
    expect(hasMetricOverviewData(data, false)).toBe(true);
    expect(metricOverviewTiles(data, false).map((tile) => tile.value)).toEqual(["500 ms", "10", "1.5 MiB", "125.5%"]);
    expect(hasMetricOverviewData(data, true)).toBe(true);
    expect(metricOverviewTiles(data, true).map((tile) => tile.value)).toEqual(["3", "1.3", "—", "—"]);
  });
});
