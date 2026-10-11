import { EDGE_STRIDE, type InfrastructureSnapshot, type TopologyView } from "@karbird/web-engine";

export const ENGINE_ROLLING_WINDOW_NS = 30_000_000_000n;

export interface MetricAggregate {
  usage: number | null;
  capacity: number | null;
  ratio: number | null;
  denominator: "host-logical-cores" | "container-limit" | "host-memory" | "mixed" | null;
  measured: number;
  total: number;
}

export interface ServiceCardMetrics {
  cpu: MetricAggregate;
  memory: MetricAggregate;
  network: {
    receiveBytesPerSecond: number | null;
    transmitBytesPerSecond: number | null;
    receiveMeasured: number;
    transmitMeasured: number;
    total: number;
  };
  requests: { incomingPerMinute: number | null; outgoingPerMinute: number | null };
}

type MetricFrame = { edgeData: Float32Array; edgeIndices: Uint32Array };
type MetricTopology = Pick<TopologyView, "nodes" | "edges">;

function finitePositive(value: number | null | undefined): value is number {
  return value !== null && value !== undefined && Number.isFinite(value) && value > 0;
}

function finiteNonNegative(value: number | null | undefined): value is number {
  return value !== null && value !== undefined && Number.isFinite(value) && value >= 0;
}

function aggregate(usage: number, capacity: number, measured: number, total: number, denominator: MetricAggregate["denominator"]): MetricAggregate {
  return {
    usage,
    capacity,
    ratio: capacity > 0 ? usage / capacity : null,
    denominator,
    measured,
    total,
  };
}

function unavailableAggregate(total: number): MetricAggregate {
  return { usage: null, capacity: null, ratio: null, denominator: null, measured: 0, total };
}

function resourceMetrics(serviceId: string, snapshot: InfrastructureSnapshot): Pick<ServiceCardMetrics, "cpu" | "memory" | "network"> {
  const instances = snapshot.instances.filter((instance) => instance.service_id === serviceId);
  const containers = snapshot.hosts.flatMap((host) => host.containers
    .filter((container) => container.service_id === serviceId)
    .map((container) => ({ host, container })));
  const total = Math.max(instances.length, containers.length);

  let cpuUsage = 0;
  let cpuCapacity = 0;
  let cpuMeasured = 0;
  const cpuHosts = new Set<string>();

  let memoryUsage = 0;
  let memoryCapacity = 0;
  let memoryMeasured = 0;
  let containerLimitCount = 0;
  let hostMemoryCount = 0;
  const fallbackMemoryHosts = new Set<string>();

  let receiveBytesPerSecond = 0;
  let transmitBytesPerSecond = 0;
  let receiveMeasured = 0;
  let transmitMeasured = 0;

  for (const { host, container } of containers) {
    if (finiteNonNegative(container.cpu_cores) && finitePositive(host.cpu_cores)) {
      cpuUsage += container.cpu_cores;
      if (!cpuHosts.has(host.id)) {
        cpuCapacity += host.cpu_cores;
        cpuHosts.add(host.id);
      }
      cpuMeasured += 1;
    }

    if (finiteNonNegative(container.memory_used_bytes)) {
      const limit = finitePositive(container.memory_limit_bytes) ? container.memory_limit_bytes : null;
      const hostCapacity = finitePositive(host.memory_total_bytes) ? host.memory_total_bytes : null;
      if (limit !== null) {
        memoryCapacity += limit;
        containerLimitCount += 1;
        memoryUsage += container.memory_used_bytes;
        memoryMeasured += 1;
      } else if (hostCapacity !== null) {
        memoryUsage += container.memory_used_bytes;
        memoryMeasured += 1;
        if (!fallbackMemoryHosts.has(host.id)) {
          memoryCapacity += hostCapacity;
          fallbackMemoryHosts.add(host.id);
          hostMemoryCount += 1;
        }
      }
    }

    if (finiteNonNegative(container.network_rx_bytes_per_sec)) {
      receiveBytesPerSecond += container.network_rx_bytes_per_sec;
      receiveMeasured += 1;
    }
    if (finiteNonNegative(container.network_tx_bytes_per_sec)) {
      transmitBytesPerSecond += container.network_tx_bytes_per_sec;
      transmitMeasured += 1;
    }
  }

  const denominator: MetricAggregate["denominator"] = containerLimitCount > 0 && hostMemoryCount > 0
    ? "mixed"
    : containerLimitCount > 0
      ? "container-limit"
      : hostMemoryCount > 0
        ? "host-memory"
        : null;

  return {
    cpu: cpuMeasured > 0 ? aggregate(cpuUsage, cpuCapacity, cpuMeasured, total, "host-logical-cores") : unavailableAggregate(total),
    memory: memoryMeasured > 0 ? aggregate(memoryUsage, memoryCapacity, memoryMeasured, total, denominator) : unavailableAggregate(total),
    network: {
      receiveBytesPerSecond: receiveMeasured > 0 ? receiveBytesPerSecond : null,
      transmitBytesPerSecond: transmitMeasured > 0 ? transmitBytesPerSecond : null,
      receiveMeasured,
      transmitMeasured,
      total,
    },
  };
}

function requestRates(
  serviceId: string,
  topology: MetricTopology,
  frame: MetricFrame,
  rollingWindowNs: bigint,
): ServiceCardMetrics["requests"] {
  if (rollingWindowNs <= 0n) return { incomingPerMinute: null, outgoingPerMinute: null };
  const serviceIndices = new Set(topology.nodes.filter((node) => node.id === serviceId && node.kind === "service").map((node) => node.index));
  if (serviceIndices.size === 0) return { incomingPerMinute: null, outgoingPerMinute: null };
  const nodesByIndex = new Map(topology.nodes.map((node) => [node.index, node]));
  const edgesByIndex = new Map(topology.edges.map((edge) => [edge.index, edge]));
  let incoming = 0;
  let outgoing = 0;
  let foundServiceEdge = false;

  for (let i = 0; i < frame.edgeIndices.length; i += 1) {
    const edgeIndex = frame.edgeIndices[i];
    const edge = edgesByIndex.get(edgeIndex ?? -1);
    if (!edge) continue;
    const source = nodesByIndex.get(edge.source);
    const target = nodesByIndex.get(edge.target);
    if (source?.kind !== "service" || target?.kind !== "service") continue;
    if (!serviceIndices.has(edge.source) && !serviceIndices.has(edge.target)) continue;
    const calls = frame.edgeData[i * EDGE_STRIDE + 5];
    if (!finiteNonNegative(calls)) continue;
    foundServiceEdge = true;
    if (serviceIndices.has(edge.target)) incoming += calls;
    if (serviceIndices.has(edge.source)) outgoing += calls;
  }

  if (!foundServiceEdge) return { incomingPerMinute: null, outgoingPerMinute: null };
  const multiplier = 60_000_000_000 / Number(rollingWindowNs);
  return { incomingPerMinute: incoming * multiplier, outgoingPerMinute: outgoing * multiplier };
}

export function deriveServiceCardMetrics(
  serviceId: string,
  topology: MetricTopology,
  frame: MetricFrame,
  snapshot: InfrastructureSnapshot,
  rollingWindowNs: bigint = ENGINE_ROLLING_WINDOW_NS,
): ServiceCardMetrics {
  return {
    ...resourceMetrics(serviceId, snapshot),
    requests: requestRates(serviceId, topology, frame, rollingWindowNs),
  };
}

export function utilizationBand(ratio: number | null): "healthy" | "warning" | "critical" | "unavailable" {
  if (ratio === null || !Number.isFinite(ratio) || ratio < 0) return "unavailable";
  if (ratio >= 0.9) return "critical";
  if (ratio >= 0.7) return "warning";
  return "healthy";
}

export function formatBytesPerSecond(value: number | null): string {
  if (value === null || !Number.isFinite(value) || value < 0) return "—";
  const units = ["B/s", "KiB/s", "MiB/s", "GiB/s"];
  let scaled = value;
  let unit = 0;
  while (scaled >= 1024 && unit < units.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  const digits = unit === 0 ? 0 : 1;
  return `${scaled.toFixed(digits)} ${units[unit]}`;
}

export function formatBytes(value: number | null): string {
  if (value === null || !Number.isFinite(value) || value < 0) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let scaled = value;
  let unit = 0;
  while (scaled >= 1024 && unit < units.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  return `${scaled.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}
