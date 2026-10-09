import { describe, expect, it } from "vitest";
import type { Schemas } from "@/lib/api";
import { summaryTracks } from "./summary-tracks";

describe("summaryTracks", () => {
  it("maps bounded overview buckets into the timeline renderer shape", () => {
    const summary: Schemas["TimelineSummary"] = {
      from_ns: "1000000000",
      to_ns: "3000000000",
      bucket_ns: "1000000000",
      buckets: 2,
      signals: [
        { requests: 4, errors: 1, p95_ms: 8, error_logs: 2, events: 3 },
        { requests: 0, errors: 0, p95_ms: null, error_logs: 0, events: 0 },
      ],
      services: [{ service_id: "svc-orders", service_name: "orders", spans: [7, 0] }],
      hosts: [],
    };

    expect(summaryTracks(summary, new Map([["svc-orders", 5]]))).toMatchObject({
      from_ns: "1000000000",
      to_ns: "3000000000",
      buckets: 2,
      requests: [4, 0],
      errors: [3, 0],
      events: [3, 0],
      latency_p95_ms: [8, null],
      latency_samples: [4, 0],
      services: [{ node: 5, name: "orders", counts: [7, 0], spans_in_view: 7, clipped: false }],
      clips: [],
      markers: [],
    });
  });

  it("omits services that are not in the current topology", () => {
    const summary: Schemas["TimelineSummary"] = {
      from_ns: "1", to_ns: "2", bucket_ns: "1", buckets: 1,
      signals: [{ requests: 0, errors: 0, p95_ms: null, error_logs: 0, events: 0 }],
      services: [{ service_id: "unknown", service_name: "unknown", spans: [9] }],
      hosts: [],
    };
    expect(summaryTracks(summary, new Map()).services).toEqual([]);
  });
});
