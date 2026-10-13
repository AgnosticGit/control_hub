import type { InfrastructureSnapshot } from "@karbird/web-engine";

/** Unique observed service instances, independent of whether machine frames are visible. */
export function serviceInstanceCounts(snapshot: InfrastructureSnapshot | null): Map<string, number> {
  const instances = new Map<string, Set<string>>();
  for (const instance of snapshot?.instances ?? []) {
    if (!instance.service_id || !instance.instance_id) continue;
    let ids = instances.get(instance.service_id);
    if (!ids) {
      ids = new Set();
      instances.set(instance.service_id, ids);
    }
    ids.add(instance.instance_id);
  }
  return new Map([...instances].map(([serviceId, ids]) => [serviceId, ids.size]));
}
