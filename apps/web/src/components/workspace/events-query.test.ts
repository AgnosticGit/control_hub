import { describe, expect, it } from "vitest";
import { filterEvents, eventAttributeText, type EventFilters, type EventRecord } from "./events-query";

const records: EventRecord[] = [
  { id: "1", time_ns: "1", observed_time_ns: "1", service_id: "svc-orders", service_name: "orders", event_type: "checkout.failed", attributes: { reason: "declined", code: "402" } },
  { id: "2", time_ns: "2", observed_time_ns: "2", service_id: "svc-inventory", service_name: "inventory", event_type: "stock.reserved", attributes: { sku: "book-7" } },
];

describe("event feed filters", () => {
  it("filters by event type and searches service names and attributes", () => {
    const filters: EventFilters = { query: "declined", eventType: "checkout.failed", serviceName: null };
    expect(filterEvents(records, filters).map((event) => event.id)).toEqual(["1"]);
    expect(filterEvents(records, { ...filters, query: "inventory", eventType: "all" }).map((event) => event.id)).toEqual(["2"]);
  });

  it("formats attributes in a stable readable order", () => {
    expect(eventAttributeText({ status: "ok", duration_ms: "12" })).toBe("duration_ms: 12 · status: ok");
  });
});
