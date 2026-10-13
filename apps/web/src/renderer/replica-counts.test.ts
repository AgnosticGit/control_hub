import { describe, expect, it } from "vitest";
import type { InfrastructureSnapshot } from "@karbird/web-engine";
import { serviceInstanceCounts } from "./replica-counts";

describe("service instance counts", () => {
  it("counts unique instances independently of machine visibility", () => {
    const snapshot = {
      instances: [
        { service_id: "catalog", instance_id: "catalog-1" },
        { service_id: "catalog", instance_id: "catalog-2" },
        { service_id: "catalog", instance_id: "catalog-2" },
        { service_id: "orders", instance_id: "orders-1" },
      ],
    } as InfrastructureSnapshot;

    expect(serviceInstanceCounts(snapshot)).toEqual(new Map([["catalog", 2], ["orders", 1]]));
  });

  it("returns no counts without an infrastructure snapshot", () => {
    expect(serviceInstanceCounts(null)).toEqual(new Map());
  });
});
