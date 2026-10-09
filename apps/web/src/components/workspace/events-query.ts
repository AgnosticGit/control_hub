import type { Schemas } from "@/lib/api";

export type EventRecord = Schemas["EventDto"];

export interface EventFilters {
  query: string;
  eventType: string;
  serviceName: string | null;
}

export function filterEvents(events: readonly EventRecord[], filters: EventFilters): EventRecord[] {
  const query = filters.query.trim().toLocaleLowerCase();
  return events.filter((event) => {
    if (filters.serviceName && event.service_name !== filters.serviceName) return false;
    if (filters.eventType !== "all" && event.event_type !== filters.eventType) return false;
    if (!query) return true;
    return [event.event_type, event.service_name, ...Object.entries(event.attributes).flat()].some((value) =>
      value.toLocaleLowerCase().includes(query),
    );
  });
}

export function eventAttributeText(attributes: Record<string, string>): string {
  return Object.entries(attributes)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key}: ${value}`)
    .join(" · ");
}
