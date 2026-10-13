import { describe, expect, it } from "vitest";
import { incidentMomentHref, parseIncidentMoment, serviceLabel } from "./model";

describe("incident moment link", () => {
  it("keeps nanosecond time exact and scopes the destination", () => {
    const href = incidentMomentHref("p 1", "prod", "1790000000000000001", "svc/a");
    expect(href).toBe("/app/p%201/prod?incident_at=1790000000000000001&service=svc%2Fa");
    expect(parseIncidentMoment("?incident_at=1790000000000000001&service=svc%2Fa")).toEqual({
      at: 1790000000000000001n,
      serviceId: "svc/a",
    });
  });

  it("rejects invalid or overflowing timestamps", () => {
    expect(parseIncidentMoment("?incident_at=abc&service=svc")).toBeNull();
    expect(parseIncidentMoment("?incident_at=9223372036854775808&service=svc")).toBeNull();
    expect(parseIncidentMoment("?incident_at=5&service=")).toBeNull();
  });
});

describe("incident service label", () => {
  it("shows the observed name and keeps the ID as a fallback", () => {
    const services = [{ service_id: "svc_42", name: "checkout" }];
    expect(serviceLabel("svc_42", services)).toBe("checkout");
    expect(serviceLabel("svc_gone", services)).toBe("svc_gone");
  });
});
