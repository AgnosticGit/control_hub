import { describe, expect, it } from "vitest";
import type { ServiceCardMetrics } from "./service-card-metrics";
import { serviceMetricPresentation } from "./service-card-presentation";

const emptyMetrics: ServiceCardMetrics = {
  cpu: { usage: null, capacity: null, ratio: null, denominator: null, measured: 0, total: 2 },
  memory: { usage: null, capacity: null, ratio: null, denominator: null, measured: 0, total: 2 },
  network: { receiveBytesPerSecond: null, transmitBytesPerSecond: null, receiveMeasured: 0, transmitMeasured: 0, total: 2 },
  requests: { incomingPerMinute: null, outgoingPerMinute: null },
};

describe("serviceMetricPresentation", () => {
  it("shows in-card placeholders while metrics are loading", () => {
    expect(serviceMetricPresentation(null, true)).toEqual({ loading: true, rows: [], loadingRows: ["cpu", "memory", "receive", "transmit", "incoming", "outgoing"] });
  });

  it("hides uncollected metric rows after loading instead of presenting em-dashes", () => {
    expect(serviceMetricPresentation(emptyMetrics, false)).toEqual({ loading: false, rows: [], loadingRows: [] });
  });

  it("keeps only measurements that were actually collected", () => {
    const metrics = {
      ...emptyMetrics,
      cpu: { usage: 0.5, capacity: 1, ratio: 0.5, denominator: "host-logical-cores" as const, measured: 1, total: 1 },
      network: { ...emptyMetrics.network, receiveBytesPerSecond: 1024, receiveMeasured: 1, total: 1 },
      requests: { incomingPerMinute: 12, outgoingPerMinute: null },
    };
    expect(serviceMetricPresentation(metrics, false).rows).toEqual(["cpu", "receive", "incoming"]);
    expect(serviceMetricPresentation(metrics, true).loadingRows).toEqual(["memory", "transmit", "outgoing"]);
  });
});
