"use client";

import type { HostDto, InfrastructureSnapshot, NoisyNeighborHint } from "@karbird/web-engine";
import { Icon } from "@/components/ui/Icon";
import { formatClock } from "@/lib/time";
import {
  MATCH_TEXT,
  RESOURCE_MATCH_TEXT,
  containersByLoad,
  formatBytes,
  formatRate,
  hintText,
  hintsForHost,
  hintsForService,
  hostLabel,
  instancesOnHost,
  percent,
  resourceHeadline,
  resourceWhere,
  serviceChain,
  usedShare,
} from "./infrastructure";

export type SignalTab = "traces" | "logs";

/** Jump to traces or logs at the selected time, optionally for one service. */
export type OpenSignals = (tab: SignalTab, serviceId: string | null) => void;

export function SignalLinks({ serviceId, onSignals, label }: { serviceId: string | null; onSignals: OpenSignals; label: string }) {
  return (
    <div className="flex flex-wrap gap-2" role="group" aria-label={`Signals for ${label}`}>
      <button type="button" className="btn btn-sm" onClick={() => onSignals("traces", serviceId)}>
        <Icon name="activity" className="h-3.5 w-3.5" />
        Traces at this moment
      </button>
      <button type="button" className="btn btn-sm" onClick={() => onSignals("logs", serviceId)}>
        <Icon name="list" className="h-3.5 w-3.5" />
        Logs at this moment
      </button>
    </div>
  );
}

function HintList({ hints, onSelect }: { hints: NoisyNeighborHint[]; onSelect: (serviceId: string) => void }) {
  if (hints.length === 0) return null;
  return (
    <ul className="space-y-2" aria-label="Noisy neighbour">
      {hints.map((h) => (
        <li key={`${h.instance_id}-${h.neighbor_container_id}`} className="rounded-lg border border-warn/50 bg-warn/10 p-2.5 text-xs">
          <div className="flex items-center gap-1.5 font-medium text-warn">
            <Icon name="alert" className="h-3.5 w-3.5" />
            Noisy neighbour
          </div>
          <p className="mt-1">{hintText(h)}</p>
          <p className="mt-1 text-muted" title="Deterministic rule, no AI">
            {h.reason}
          </p>
          <div className="mt-1.5 flex gap-2">
            <button type="button" className="text-accent hover:underline" onClick={() => onSelect(h.service_id)}>
              {h.service_name}
            </button>
            {h.neighbor_service_id ? (
              <button type="button" className="text-accent hover:underline" onClick={() => onSelect(h.neighbor_service_id ?? "")}>
                {h.neighbor_name}
              </button>
            ) : null}
          </div>
        </li>
      ))}
    </ul>
  );
}

function Bar({ share, tone }: { share: number | null; tone: "accent" | "warn" | "error" }) {
  const w = share === null ? 0 : Math.round(share * 100);
  const color = tone === "error" ? "bg-error" : tone === "warn" ? "bg-warn" : "bg-accent";
  return (
    <span className="block h-1.5 w-full overflow-hidden rounded-full bg-surface-3" aria-hidden>
      <span className={`block h-full ${color}`} style={{ width: `${w}%` }} />
    </span>
  );
}

function tone(share: number | null): "accent" | "warn" | "error" {
  if (share === null) return "accent";
  return share >= 0.9 ? "error" : share >= 0.7 ? "warn" : "accent";
}

function Gauge({ label, value, sub, share }: { label: string; value: string; sub?: string; share: number | null }) {
  return (
    <div className="space-y-1 rounded-lg border border-border bg-bg/50 px-3 py-2">
      <dt className="text-[11px] text-muted">{label}</dt>
      <dd className="text-lg leading-tight font-semibold tabular-nums">{value}</dd>
      {sub ? <dd className="text-[11px] text-muted tabular-nums">{sub}</dd> : null}
      <Bar share={share} tone={tone(share)} />
    </div>
  );
}

interface HostCardProps {
  host: HostDto;
  snapshot: InfrastructureSnapshot;
  cursor: bigint;
  onClose: () => void;
  onSelect: (serviceId: string) => void;
  onSignals: OpenSignals;
}

/** Machine card: CPU, RAM, disk, network and the containers on it with their shares. */
export function HostCard({ host, snapshot, cursor, onClose, onSelect, onSignals }: HostCardProps) {
  const mem = usedShare(host.memory_used_bytes, host.memory_total_bytes);
  const disk = usedShare(host.disk_used_bytes, host.disk_total_bytes);
  const instances = instancesOnHost(snapshot, host.id);
  return (
    <div className="text-sm" aria-label={`Machine ${host.name || host.id}`}>
      <div className="flex items-start gap-3 border-b border-border p-4">
        <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-violet/15 text-violet">
          <Icon name="cpu" className="h-5 w-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="text-base font-semibold break-all">{host.name || host.id}</h2>
          <p className="text-xs text-muted">machine · {hostLabel(host)}</p>
        </div>
        <button type="button" aria-label="Close machine" onClick={onClose} className="btn btn-icon btn-sm">
          <Icon name="x" className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="space-y-4 p-4">
        <HintList hints={hintsForHost(snapshot, host.id)} onSelect={onSelect} />
        {host.has_metrics ? (
          <dl className="grid grid-cols-2 gap-2" aria-label="Machine metrics">
            <Gauge
              label="CPU"
              value={percent(host.cpu_utilization)}
              sub={host.cpu_cores !== null ? `${host.cpu_cores} cores` : undefined}
              share={host.cpu_utilization}
            />
            <Gauge
              label="RAM"
              value={`${formatBytes(host.memory_used_bytes)} / ${formatBytes(host.memory_total_bytes)}`}
              sub={percent(mem)}
              share={mem}
            />
            <Gauge label="Disk" value={`${formatBytes(host.disk_used_bytes)} / ${formatBytes(host.disk_total_bytes)}`} sub={percent(disk)} share={disk} />
            <div className="space-y-1 rounded-lg border border-border bg-bg/50 px-3 py-2">
              <dt className="text-[11px] text-muted">Network</dt>
              <dd className="text-xs tabular-nums">↓ {formatRate(host.network_rx_bytes_per_sec)}</dd>
              <dd className="text-xs tabular-nums">↑ {formatRate(host.network_tx_bytes_per_sec)}</dd>
            </div>
          </dl>
        ) : (
          <p className="rounded-lg border border-dashed border-border p-3 text-xs text-muted">
            No Karbird agent reports from this machine. It is known only from the host.name of its services&apos; spans, so
            CPU, memory, disk and containers are unknown.
          </p>
        )}

        <section aria-label="Containers">
          <h3 className="label">Containers ({host.containers.length})</h3>
          {host.containers.length === 0 ? <p className="mt-1 text-xs text-muted">No container metrics in the window.</p> : null}
          <ul className="mt-2 space-y-2">
            {containersByLoad(host).map((c) => (
              <li key={c.id} className="rounded-lg border border-border bg-bg/50 p-2.5 text-xs">
                <div className="flex items-center gap-2">
                  <span className="truncate font-medium" title={c.id}>
                    {c.name}
                  </span>
                  {c.service_id ? (
                    <button type="button" className="text-accent hover:underline" onClick={() => onSelect(c.service_id ?? "")}>
                      {c.service_name}
                    </button>
                  ) : null}
                  {c.resource_id ? (
                    <button type="button" className="text-accent hover:underline" onClick={() => onSelect(c.resource_id ?? "")}>
                      {c.resource_name ?? c.resource_id}
                    </button>
                  ) : null}
                  <span className="ml-auto tabular-nums" aria-label={`${c.name} CPU share`}>
                    CPU {percent(c.cpu_share)}
                  </span>
                </div>
                {c.image ? (
                  <p className="mt-0.5 truncate font-mono text-[10px] text-muted" title="container.image.name">
                    {c.image}
                  </p>
                ) : null}
                <div className="mt-1.5 grid grid-cols-[3rem_1fr_3rem] items-center gap-x-2 gap-y-1 text-[11px] text-muted">
                  <span>CPU</span>
                  <Bar share={c.cpu_share} tone={tone(c.cpu_share)} />
                  <span className="text-right tabular-nums">{percent(c.cpu_share)}</span>
                  <span>RAM</span>
                  <Bar share={c.memory_share} tone={tone(c.memory_share)} />
                  <span className="text-right tabular-nums">{percent(c.memory_share)}</span>
                </div>
                {c.service_id ? (
                  <div className="mt-1.5">
                    <SignalLinks serviceId={c.service_id} onSignals={onSignals} label={c.name} />
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        </section>

        {instances.length > 0 ? (
          <section aria-label="Instances on this machine">
            <h3 className="label">Service instances</h3>
            <ul className="mt-2 space-y-1 text-xs">
              {instances.map((i) => (
                <li key={i.instance_id} className="flex items-center gap-2">
                  <button type="button" className="font-medium hover:text-accent" onClick={() => onSelect(i.service_id)}>
                    {i.service_name}
                  </button>
                  <span className="truncate font-mono text-[10px] text-muted">{i.instance_id}</span>
                </li>
              ))}
            </ul>
          </section>
        ) : null}

        <SignalLinks serviceId={null} onSignals={onSignals} label={host.name || host.id} />
        <p className="text-[11px] text-muted">
          At {formatClock(cursor)}; machine metrics over the last {Number(BigInt(snapshot.window_ns) / 1_000_000_000n)} s.
        </p>
      </div>
    </div>
  );
}

interface ResourceRunsOnProps {
  nodeId: string;
  snapshot: InfrastructureSnapshot;
  onSelectHost: (hostId: string) => void;
}

/** Database or queue → container → machine, or why it is external. */
export function ResourceRunsOn({ nodeId, snapshot, onSelectHost }: ResourceRunsOnProps) {
  const where = resourceWhere(snapshot, nodeId);
  if (!where) {
    return (
      <section aria-label="Runs on" className="space-y-1">
        <h3 className="label">Runs on</h3>
        <p className="text-xs text-muted">No service called it in the last 5 minutes.</p>
      </section>
    );
  }
  const { placement, host, container } = where;
  const how = RESOURCE_MATCH_TEXT[placement.match_reason];
  return (
    <section aria-label="Runs on" className="space-y-2">
      <h3 className="label">Runs on</h3>
      <div className="rounded-lg border border-border bg-bg/50 p-2.5 text-xs">
        <div className="flex flex-wrap items-center gap-1.5">
          {placement.location === "internal" ? (
            <>
              <span className="badge border border-accent/50 text-accent">Internal</span>
              {host ? (
                <button type="button" className="font-medium text-accent hover:underline" onClick={() => onSelectHost(host.id)}>
                  {host.name || host.id}
                </button>
              ) : (
                <span>{placement.host_id}</span>
              )}
              {container ? (
                <>
                  <Icon name="arrowRight" className="h-3 w-3 text-muted" />
                  <span title={container.id}>container {container.name}</span>
                </>
              ) : null}
            </>
          ) : (
            <span className="badge border border-dashed border-current text-text">{resourceHeadline(where)}</span>
          )}
        </div>
        {container ? (
          <p className="mt-1 text-[11px] text-muted tabular-nums">
            CPU {percent(container.cpu_share)} of machine · RAM {formatBytes(container.memory_used_bytes)}
            {container.image ? ` · ${container.image}` : ""}
          </p>
        ) : null}
        <p className="mt-1 text-[11px] text-muted">{placement.reason}</p>
        {how ? <p className="mt-1 text-[11px] text-muted">{how}</p> : null}
        {placement.addresses.length > 0 ? (
          <p className="mt-1 font-mono text-[10px] text-muted" aria-label="Addresses services connect to">
            {placement.addresses.join(", ")}
          </p>
        ) : null}
      </div>
    </section>
  );
}

interface RunsOnProps {
  serviceId: string;
  snapshot: InfrastructureSnapshot;
  onSelect: (serviceId: string) => void;
  onSelectHost: (hostId: string) => void;
}

/** Service → instances → container → machine. */
export function RunsOn({ serviceId, snapshot, onSelect, onSelectHost }: RunsOnProps) {
  const chain = serviceChain(snapshot, serviceId);
  return (
    <section aria-label="Runs on" className="space-y-2">
      <HintList hints={hintsForService(snapshot, serviceId)} onSelect={onSelect} />
      <h3 className="label">Runs on ({chain.length === 1 ? "1 instance" : `${chain.length} instances`})</h3>
      {chain.length === 0 ? <p className="text-xs text-muted">No request spans from this service in the last 5 minutes.</p> : null}
      <ol className="space-y-2">
        {chain.map(({ instance, container, host }) => (
          <li key={instance.instance_id} className="rounded-lg border border-border bg-bg/50 p-2.5 text-xs">
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="font-mono" title="service.instance.id">
                {instance.instance_id}
              </span>
              <Icon name="arrowRight" className="h-3 w-3 text-muted" />
              <span title={container?.id ?? undefined}>{container ? `container ${container.name}` : "no container"}</span>
              <Icon name="arrowRight" className="h-3 w-3 text-muted" />
              {host ? (
                <button type="button" className="font-medium text-accent hover:underline" onClick={() => onSelectHost(host.id)}>
                  {host.name || host.id}
                </button>
              ) : (
                <span className="text-muted">unknown machine</span>
              )}
            </div>
            <p className="mt-1 text-[11px] text-muted">
              {MATCH_TEXT[instance.match_reason]}
              {instance.p95_ms !== null ? ` · p95 ${Math.round(instance.p95_ms)} ms` : ""}
              {container?.cpu_share !== null && container?.cpu_share !== undefined ? ` · CPU ${percent(container.cpu_share)} of machine` : ""}
            </p>
          </li>
        ))}
      </ol>
    </section>
  );
}
