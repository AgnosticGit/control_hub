import { describe, expect, it } from "vitest";
import type { ContainerDto, HostDto, InfrastructureSnapshot, TopologyView, TopologyViewEdge } from "@karbird/web-engine";
import { EDGE_STRIDE } from "@karbird/web-engine";
import { deriveServiceCardMetrics, formatBytesPerSecond, utilizationBand } from "./service-card-metrics";

const container = (id: string, over: Partial<ContainerDto> = {}): ContainerDto => ({
  id,
  name: id,
  service_id: "svc_orders",
  service_name: "orders",
  image: null,
  compose_service: null,
  resource_id: null,
  resource_name: null,
  cpu_cores: null,
  cpu_share: null,
  memory_used_bytes: null,
  memory_limit_bytes: null,
  memory_share: null,
  network_rx_bytes_per_sec: null,
  network_tx_bytes_per_sec: null,
  ...over,
});

const host = (id: string, containers: ContainerDto[], over: Partial<HostDto> = {}): HostDto => ({
  id,
  name: id,
  has_metrics: true,
  cpu_utilization: null,
  cpu_cores: 4,
  memory_used_bytes: null,
  memory_total_bytes: 8 * 1024 ** 3,
  disk_used_bytes: null,
  disk_total_bytes: null,
  network_rx_bytes_per_sec: null,
  network_tx_bytes_per_sec: null,
  containers,
  last_seen_ns: null,
  ...over,
});

const edge = (index: number, source: number, target: number): TopologyViewEdge => ({
  index, source, target, relation: "calls", provenance: "observed", reasons: [], points: [],
});

const baseTopology: Pick<TopologyView, "nodes" | "edges"> = {
  nodes: [
    { index: 0, id: "svc_gateway", name: "gateway", kind: "service", provenance: "observed", x: 0, y: 0 },
    { index: 1, id: "svc_orders", name: "orders", kind: "service", provenance: "observed", x: 0, y: 0 },
    { index: 2, id: "external_client", name: "client", kind: "external_client", provenance: "observed", x: 0, y: 0 },
    { index: 3, id: "db", name: "database", kind: "database", provenance: "observed", x: 0, y: 0 },
  ],
  edges: [edge(0, 0, 1), edge(1, 1, 0), edge(2, 2, 1), edge(3, 1, 3)],
};

function snapshot(hosts: HostDto[], instances = 2): InfrastructureSnapshot {
  return {
    at_ns: "0",
    window_ns: "10000000000",
    hosts,
    instances: Array.from({ length: instances }, (_, index) => ({
      service_id: "svc_orders", service_name: "orders", instance_id: `orders-${index + 1}`,
      host_id: hosts[index % Math.max(hosts.length, 1)]?.id ?? null,
      container_id: `orders-${index + 1}`, match_reason: "container_id", p95_ms: null, baseline_p95_ms: null, requests: 0,
    })),
    hints: [], resources: [], notes: [],
  };
}

function frameCalls(calls: number[]): { edgeData: Float32Array; edgeIndices: Uint32Array } {
  const edgeData = new Float32Array(calls.length * EDGE_STRIDE);
  calls.forEach((count, index) => { edgeData[index * EDGE_STRIDE + 5] = count; });
  return { edgeData, edgeIndices: Uint32Array.from(calls.map((_, index) => index)) };
}

describe("deriveServiceCardMetrics", () => {
  it("aggregates replica utilization from summed usage and unique known host capacity", () => {
    const result = deriveServiceCardMetrics("svc_orders", baseTopology, frameCalls([30, 10, 50, 20]), snapshot([
      host("host-a", [container("orders-1", { cpu_cores: 1, memory_used_bytes: 2_000, memory_limit_bytes: 4_000, network_rx_bytes_per_sec: 100, network_tx_bytes_per_sec: 50 })]),
      host("host-b", [container("orders-2", { cpu_cores: 2, memory_used_bytes: 4_000, memory_limit_bytes: 8_000, network_rx_bytes_per_sec: 200, network_tx_bytes_per_sec: 150 })]),
    ]));

    expect(result.cpu).toMatchObject({ usage: 3, capacity: 8, ratio: 3 / 8, denominator: "host-logical-cores", measured: 2, total: 2 });
    expect(result.memory).toMatchObject({ usage: 6_000, capacity: 12_000, ratio: 0.5, denominator: "container-limit", measured: 2, total: 2 });
    expect(result.network).toEqual({ receiveBytesPerSecond: 300, transmitBytesPerSecond: 200, receiveMeasured: 2, transmitMeasured: 2, total: 2 });
  });

  it("marks partial data and uses host memory capacity when a container limit is unavailable", () => {
    const result = deriveServiceCardMetrics("svc_orders", baseTopology, frameCalls([]), snapshot([
      host("host-a", [container("orders-1", { cpu_cores: 0.5, memory_used_bytes: 1_000, memory_limit_bytes: null })]),
    ]));

    expect(result.cpu).toMatchObject({ usage: 0.5, capacity: 4, ratio: 0.125, measured: 1, total: 2 });
    expect(result.memory).toMatchObject({ usage: 1_000, capacity: 8 * 1024 ** 3, denominator: "host-memory", measured: 1, total: 2 });
    expect(result.network).toMatchObject({ receiveBytesPerSecond: null, transmitBytesPerSecond: null, receiveMeasured: 0, transmitMeasured: 0, total: 2 });
  });

  it("keeps missing measurements and zero capacities unavailable instead of showing zero", () => {
    const result = deriveServiceCardMetrics("svc_orders", baseTopology, frameCalls([]), snapshot([
      host("host-a", [container("orders-1", { cpu_cores: 1, memory_used_bytes: 5, memory_limit_bytes: 0 })], { cpu_cores: 0, memory_total_bytes: 0 }),
    ]));

    expect(result.cpu.ratio).toBeNull();
    expect(result.cpu.usage).toBeNull();
    expect(result.cpu.capacity).toBeNull();
    expect(result.memory.ratio).toBeNull();
    expect(result.memory.usage).toBeNull();
    expect(result.memory.capacity).toBeNull();
    expect(result.network.receiveBytesPerSecond).toBeNull();
  });

  it("does not report zero requests when no service-to-service edge is available", () => {
    const result = deriveServiceCardMetrics("svc_orders", baseTopology, frameCalls([]), snapshot([], 0));
    expect(result.requests).toEqual({ incomingPerMinute: null, outgoingPerMinute: null });
  });

  it("converts service-to-service edge calls over the rolling window to incoming and outgoing per minute", () => {
    const result = deriveServiceCardMetrics("svc_orders", baseTopology, frameCalls([30, 10, 50, 20]), snapshot([], 0), 30_000_000_000n);

    expect(result.requests).toEqual({ incomingPerMinute: 60, outgoingPerMinute: 20 });
  });

  it("does not derive rates for a nonpositive rolling window", () => {
    const result = deriveServiceCardMetrics("svc_orders", baseTopology, frameCalls([30, 10, 50, 20]), snapshot([], 0), 0n);

    expect(result.requests).toEqual({ incomingPerMinute: null, outgoingPerMinute: null });
  });
});

describe("metric presentation", () => {
  it("uses green, yellow, and red at the agreed utilization thresholds", () => {
    expect(utilizationBand(0.6999)).toBe("healthy");
    expect(utilizationBand(0.7)).toBe("warning");
    expect(utilizationBand(0.8999)).toBe("warning");
    expect(utilizationBand(0.9)).toBe("critical");
    expect(utilizationBand(null)).toBe("unavailable");
  });

  it("formats network rates without pretending they have a saturation limit", () => {
    expect(formatBytesPerSecond(500)).toBe("500 B/s");
    expect(formatBytesPerSecond(1_536)).toBe("1.5 KiB/s");
    expect(formatBytesPerSecond(1_572_864)).toBe("1.5 MiB/s");
    expect(formatBytesPerSecond(null)).toBe("—");
  });
});
