/** Links from a persisted incident to the exact historical moment in the workspace. */
export function incidentMomentHref(projectId: string, environmentId: string, atNs: string, serviceId: string): string {
  return `/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}`
    + `?incident_at=${encodeURIComponent(atNs)}&service=${encodeURIComponent(serviceId)}`;
}

export function parseIncidentMoment(search: string): { at: bigint; serviceId: string } | null {
  const params = new URLSearchParams(search);
  const raw = params.get("incident_at");
  const serviceId = params.get("service");
  if (!raw || !/^\d{1,19}$/.test(raw) || !serviceId || serviceId.length > 200 || /[\x00-\x1f\x7f]/.test(serviceId)) return null;
  const at = BigInt(raw);
  if (at <= 0n || at > 9223372036854775807n) return null;
  return { at, serviceId };
}

export function serviceLabel(serviceId: string, services: ReadonlyArray<{ service_id: string; name: string }> | undefined): string {
  return services?.find((service) => service.service_id === serviceId)?.name ?? serviceId;
}
