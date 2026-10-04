import { describe, expect, it } from "vitest";
import type { ContainerDto, HostDto, InfrastructureSnapshot, InstanceDto, NoisyNeighborHint } from "@karbird/web-engine";
import {
  INFRA_STEP_NS,
  containersByLoad,
  formatBytes,
  formatRate,
  hintText,
  hintsForHost,
  hintsForService,
  hostLabel,
  infraAt,
  instancesOnHost,
  percent,
  serviceChain,
  usedShare,
} from "./infrastructure";

const MIB = 1_048_576;

function container(id: string, name: string, service: string | null, cpu: number | null, mem: number | null = null): ContainerDto {
  return {
    id,
    name,
    service_id: service ? `svc_${service}` : null,
    service_name: service,
    cpu_cores: cpu,
    cpu_share: cpu,
    memory_used_bytes: null,
    memory_limit_bytes: null,
    memory_share: mem,
    network_rx_bytes_per_sec: null,
    network_tx_bytes_per_sec: null,
  };
}

function host(id: string, over: Partial<HostDto> = {}): HostDto {
  return {
    id,
    name: id,
    has_metrics: true,
    cpu_utilization: 0.92,
    cpu_cores: 1,
    memory_used_bytes: 870 * MIB,
    memory_total_bytes: 1024 * MIB,
    disk_used_bytes: 10 * 1024 * MIB,
    disk_total_bytes: 25 * 1024 * MIB,
    network_rx_bytes_per_sec: 2048,
    network_tx_bytes_per_sec: 512,
    containers: [],
    last_seen_ns: "1",
    ...over,
  };
}

function instance(service: string, id: string, hostId: string | null, containerId: string | null): InstanceDto {
  return {
    service_id: `svc_${service}`,
    service_name: service,
    instance_id: id,
    host_id: hostId,
    container_id: containerId,
    match_reason: containerId ? "container_id" : hostId ? "host_name" : "unknown",
    p95_ms: 10,
    baseline_p95_ms: 5,
    requests: 100,
  };
}

const hint: NoisyNeighborHint = {
  service_id: "svc_orders",
  service_name: "orders",
  instance_id: "orders-1",
  neighbor_service_id: "svc_inventory",
  neighbor_name: "inventory",
  neighbor_container_id: "c-inv",
  host_id: "vps-app",
  host_name: "vps-app",
  p95_ms: 412.4,
  baseline_p95_ms: 80.2,
  neighbor_cpu_share: 0.95,
  host_cpu_utilization: 0.97,
  reason: "rule",
};

const snap: InfrastructureSnapshot = {
  at_ns: "100",
  window_ns: "120000000000",
  hosts: [
    host("vps-app", {
      containers: [container("c-ord", "orders", "orders", 0.02), container("c-inv", "inventory", "inventory", 0.95), container("c-cat1", "catalog-1", "catalog", 0.02)],
    }),
    host("vps-worker", { cpu_utilization: 0.2, containers: [container("c-cat2", "catalog-2", "catalog", 0.1), container("c-cat3", "catalog-3", "catalog", 0.1)] }),
  ],
  instances: [
    instance("orders", "orders-1", "vps-app", "c-ord"),
    instance("catalog", "catalog-3", "vps-worker", "c-cat3"),
    instance("catalog", "catalog-1", "vps-app", "c-cat1"),
    instance("catalog", "catalog-2", "vps-worker", "c-cat2"),
    instance("ghost", "ghost-1", null, null),
  ],
  hints: [hint],
  notes: [],
};

describe("infraAt", () => {
  it("quantises the cursor to the end of a 10 s bucket", () => {
    const s = INFRA_STEP_NS;
    expect(infraAt(0n)).toBe(s);
    expect(infraAt(1n)).toBe(s);
    expect(infraAt(s - 1n)).toBe(s);
    expect(infraAt(s)).toBe(2n * s);
    expect(infraAt(1_790_000_000_123_456_789n)).toBe(1_790_000_010_000_000_000n);
  });

  it("is stable inside a bucket and never earlier than the cursor", () => {
    for (const c of [5n, 9_999_999_999n, 123_456_789_012_345n, -1n, -INFRA_STEP_NS]) {
      expect(infraAt(c)).toBeGreaterThan(c);
      expect(infraAt(c) - c).toBeLessThanOrEqual(INFRA_STEP_NS);
      expect(infraAt(c + 1n) === infraAt(c) || (c + 1n) % INFRA_STEP_NS === 0n).toBe(true);
    }
  });
});

describe("hostLabel", () => {
  it("matches the frame label on the map", () => {
    expect(hostLabel(host("vps-app"))).toBe("vps-app · CPU 92% · RAM 870/1024 MB");
  });

  it("says when a machine has no agent metrics", () => {
    expect(hostLabel(host("edge-1", { has_metrics: false, cpu_utilization: null }))).toBe("edge-1 · no agent metrics");
  });

  it("leaves out unknown values and falls back to the ID", () => {
    expect(hostLabel(host("i-0abc", { name: "", memory_total_bytes: null, cpu_utilization: 0.054 }))).toBe("i-0abc · CPU 5%");
  });
});

describe("formatting", () => {
  it("formats bytes, rates and shares, with a dash for unknown", () => {
    expect(formatBytes(870 * MIB)).toBe("870 MB");
    expect(formatBytes(25 * 1024 * MIB)).toBe("25.0 GB");
    expect(formatBytes(2048)).toBe("2 KB");
    expect(formatBytes(12)).toBe("12 B");
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(Number.NaN)).toBe("—");
    expect(formatRate(2048)).toBe("2 KB/s");
    expect(formatRate(null)).toBe("—");
    expect(percent(0.954)).toBe("95%");
    expect(percent(null)).toBe("—");
  });

  it("clamps used shares and rejects a missing or zero total", () => {
    expect(usedShare(870, 1024)).toBeCloseTo(0.85, 2);
    expect(usedShare(2, 1)).toBe(1);
    expect(usedShare(1, 0)).toBeNull();
    expect(usedShare(null, 10)).toBeNull();
  });
});

describe("containersByLoad", () => {
  it("lists the heaviest CPU consumer first and unknown shares last", () => {
    const h = host("h", { containers: [container("a", "a", null, 0.1), container("b", "b", null, null), container("c", "c", null, 0.95)] });
    expect(containersByLoad(h).map((c) => c.id)).toEqual(["c", "a", "b"]);
    expect(h.containers.map((c) => c.id)).toEqual(["a", "b", "c"]);
  });
});

describe("serviceChain", () => {
  it("links every instance to its container and machine, in instance order", () => {
    const chain = serviceChain(snap, "svc_catalog");
    expect(chain.map((l) => [l.instance.instance_id, l.container?.name, l.host?.id])).toEqual([
      ["catalog-1", "catalog-1", "vps-app"],
      ["catalog-2", "catalog-2", "vps-worker"],
      ["catalog-3", "catalog-3", "vps-worker"],
    ]);
  });

  it("keeps an unmatched instance with no container or machine", () => {
    const [ghost] = serviceChain(snap, "svc_ghost");
    expect(ghost?.container).toBeNull();
    expect(ghost?.host).toBeNull();
  });

  it("is empty without a snapshot or for an unknown service", () => {
    expect(serviceChain(null, "svc_catalog")).toEqual([]);
    expect(serviceChain(snap, "svc_nope")).toEqual([]);
  });
});

describe("hints", () => {
  it("finds a hint from the victim, the neighbour and the machine", () => {
    expect(hintsForService(snap, "svc_orders")).toHaveLength(1);
    expect(hintsForService(snap, "svc_inventory")).toHaveLength(1);
    expect(hintsForService(snap, "svc_catalog")).toHaveLength(0);
    expect(hintsForHost(snap, "vps-app")).toHaveLength(1);
    expect(hintsForHost(snap, "vps-worker")).toHaveLength(0);
    expect(hintsForHost(null, "vps-app")).toHaveLength(0);
  });

  it("explains the hint in one line", () => {
    expect(hintText(hint)).toBe("orders got slower (p95 80 → 412 ms) while inventory used 95% of CPU on vps-app");
  });

  it("lists the instances on a machine", () => {
    expect(instancesOnHost(snap, "vps-worker").map((i) => i.instance_id)).toEqual(["catalog-3", "catalog-2"]);
  });
});
