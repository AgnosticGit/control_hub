import type {
  ContainerDto,
  HostDto,
  InfrastructureSnapshot,
  InstanceDto,
  InstanceMatch,
  NoisyNeighborHint,
  ResourceMatch,
  ResourcePlacementDto,
} from "@karbird/web-engine";
import { NS_PER_SEC } from "@/lib/time";

/** Snapshots are requested on this grid so scrubbing and live ticks reuse one request. */
export const INFRA_STEP_NS = 10n * NS_PER_SEC;

/** The snapshot time for a cursor: the end of its `INFRA_STEP_NS` bucket. */
export function infraAt(cursor: bigint): bigint {
  const bucket = cursor >= 0n ? cursor / INFRA_STEP_NS : (cursor - INFRA_STEP_NS + 1n) / INFRA_STEP_NS;
  return (bucket + 1n) * INFRA_STEP_NS;
}

const MIB = 1_048_576;

function mb(bytes: number): number {
  return Math.round(bytes / MIB);
}

export function percent(share: number | null | undefined): string {
  return share === null || share === undefined || !Number.isFinite(share) ? "—" : `${Math.round(share * 100)}%`;
}

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return "—";
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (bytes >= MIB) return `${mb(bytes)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${Math.round(bytes)} B`;
}

export function formatRate(bytesPerSec: number | null | undefined): string {
  const b = formatBytes(bytesPerSec);
  return b === "—" ? b : `${b}/s`;
}

/** Same text as the machine frame on the map: `vps-app · CPU 92% · RAM 870/1024 MB`. */
export function hostLabel(h: HostDto): string {
  const name = h.name || h.id;
  if (!h.has_metrics) return `${name} · no collector metrics`;
  let label = name;
  if (h.cpu_utilization !== null) label += ` · CPU ${Math.round(h.cpu_utilization * 100)}%`;
  if (h.memory_used_bytes !== null && h.memory_total_bytes !== null) {
    label += ` · RAM ${mb(h.memory_used_bytes)}/${mb(h.memory_total_bytes)} MB`;
  }
  return label;
}

/** Used share of a total, 0..1, or null when either side is unknown. */
export function usedShare(used: number | null, total: number | null): number | null {
  if (used === null || total === null || !(total > 0)) return null;
  return Math.max(0, Math.min(1, used / total));
}

/** Containers by CPU share (then memory share), heaviest first. */
export function containersByLoad(h: HostDto): ContainerDto[] {
  const key = (c: ContainerDto) => [c.cpu_share ?? -1, c.memory_share ?? -1] as const;
  return [...h.containers].sort((a, b) => {
    const [ac, am] = key(a);
    const [bc, bm] = key(b);
    return bc - ac || bm - am || a.name.localeCompare(b.name);
  });
}

export const MATCH_TEXT: Record<InstanceMatch, string> = {
  container_id: "matched by container ID",
  container_hostname: "matched by container hostname",
  host_name: "matched by machine name",
  unverified: "machine from the span's host.name; no collector metrics to confirm it",
  unknown: "the service reports neither container.id nor host.name",
};

export interface ChainLink {
  instance: InstanceDto;
  container: ContainerDto | null;
  host: HostDto | null;
}

/** A container of the infrastructure layer: container IDs are only unique per machine. */
export interface ContainerRef {
  hostId: string;
  containerId: string;
}

export function findContainer(
  snap: InfrastructureSnapshot | null,
  ref: ContainerRef,
): { host: HostDto; container: ContainerDto } | null {
  const host = snap?.hosts.find((h) => h.id === ref.hostId);
  const container = host?.containers.find((c) => c.id === ref.containerId);
  return host && container ? { host, container } : null;
}

/** The container a service instance runs in, when the snapshot knows it. */
export function containerOfInstance(snap: InfrastructureSnapshot | null, instanceId: string, hostId: string): ContainerRef | null {
  const instance = snap?.instances.find((i) => i.instance_id === instanceId && i.host_id === hostId);
  if (!instance?.container_id || !instance.host_id) return null;
  const ref = { hostId: instance.host_id, containerId: instance.container_id };
  return findContainer(snap ?? null, ref) ? ref : null;
}

/** Service → instances → container → machine, for the inspector. */
export function serviceChain(snap: InfrastructureSnapshot | null, serviceId: string): ChainLink[] {
  if (!snap) return [];
  return snap.instances
    .filter((i) => i.service_id === serviceId)
    .map((instance) => {
      const host = snap.hosts.find((h) => h.id === instance.host_id) ?? null;
      const container = host?.containers.find((c) => c.id === instance.container_id) ?? null;
      return { instance, container, host };
    })
    .sort((a, b) => a.instance.instance_id.localeCompare(b.instance.instance_id));
}

export function hintsForService(snap: InfrastructureSnapshot | null, serviceId: string): NoisyNeighborHint[] {
  return snap?.hints.filter((h) => h.service_id === serviceId || h.neighbor_service_id === serviceId) ?? [];
}

export function hintsForHost(snap: InfrastructureSnapshot | null, hostId: string): NoisyNeighborHint[] {
  return snap?.hints.filter((h) => h.host_id === hostId) ?? [];
}

/** One-line banner text for a hint. */
export function hintText(h: NoisyNeighborHint): string {
  return `${h.service_name} got slower (p95 ${Math.round(h.baseline_p95_ms)} → ${Math.round(h.p95_ms)} ms) while ${
    h.neighbor_name
  } used ${percent(h.neighbor_cpu_share)} of CPU on ${h.host_name}`;
}

export const RESOURCE_MATCH_TEXT: Record<ResourceMatch, string> = {
  compose_service: "matched by Docker Compose service name",
  container_name: "matched by container name",
  container_hostname: "matched by container hostname",
  loopback: "connected over localhost; container found by its image",
  host_name: "matched by machine name",
  none: "",
};

export interface ResourceWhere {
  placement: ResourcePlacementDto;
  host: HostDto | null;
  container: ContainerDto | null;
}

/** Where a database or queue node runs, from the snapshot; null when the API did not see it called. */
export function resourceWhere(snap: InfrastructureSnapshot | null, nodeId: string): ResourceWhere | null {
  const placement = snap?.resources.find((r) => r.node_id === nodeId);
  if (!snap || !placement) return null;
  const host = placement.location === "internal" ? (snap.hosts.find((h) => h.id === placement.host_id) ?? null) : null;
  const container = host?.containers.find((c) => c.id === placement.container_id) ?? null;
  return { placement, host, container };
}

/** One line for the inspector: `vps-app · container postgres` / `External` / `Unknown`. */
export function resourceHeadline(w: ResourceWhere): string {
  const { placement, host, container } = w;
  if (placement.location === "external") return "External: no Karbird Collector there";
  if (placement.location === "unknown") return "Location unknown";
  const machine = host ? host.name || host.id : (placement.host_id ?? "unknown machine");
  return container ? `${machine} · container ${container.name}` : machine;
}

/** Instances of a service that run on a machine. */
export function instancesOnHost(snap: InfrastructureSnapshot | null, hostId: string): InstanceDto[] {
  return snap?.instances.filter((i) => i.host_id === hostId) ?? [];
}
