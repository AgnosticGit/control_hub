import type { ServiceCardMetrics } from "./service-card-metrics";

export type ServiceMetricRow = "cpu" | "memory" | "receive" | "transmit" | "incoming" | "outgoing";

export interface ServiceMetricPresentation {
  loading: boolean;
  rows: ServiceMetricRow[];
}

/** Keep the card quiet for measurements that the collector has not supplied. */
export function serviceMetricPresentation(
  metrics: ServiceCardMetrics | null,
  loading: boolean,
): ServiceMetricPresentation {
  if (!metrics) return { loading, rows: [] };
  const rows: ServiceMetricRow[] = [];
  if (metrics.cpu.usage !== null) rows.push("cpu");
  if (metrics.memory.usage !== null) rows.push("memory");
  if (metrics.network.receiveBytesPerSecond !== null) rows.push("receive");
  if (metrics.network.transmitBytesPerSecond !== null) rows.push("transmit");
  if (metrics.requests.incomingPerMinute !== null) rows.push("incoming");
  if (metrics.requests.outgoingPerMinute !== null) rows.push("outgoing");
  return { loading: false, rows };
}
