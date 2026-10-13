"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { AppShell } from "@/components/shell/AppShell";
import { Icon } from "@/components/ui/Icon";
import { api, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { can } from "@/lib/permissions";
import { useSession } from "@/lib/session";
import { formatDateTime, nowNs, parseNs, NS_PER_SEC } from "@/lib/time";
import { compareServices, comparisonWarnings, comparisonWindow, type ServiceComparison } from "./change-replay";

type Event = Schemas["EventDto"];
type Services = Schemas["ServicesResponse"];
const DAY_NS = 24n * 60n * 60n * NS_PER_SEC;

function count(value: number | undefined): string { return value === undefined ? "—" : value.toLocaleString(); }
function percent(value: number | null): string { return value === null ? "—" : `${value.toFixed(1)}%`; }
function latency(value: number | null | undefined): string { return value == null ? "—" : `${value.toFixed(1)} ms`; }
function version(event: Event): string | null {
  return event.attributes["service.version"] ?? event.attributes["deployment.version"] ?? event.attributes.version ?? null;
}

function Metric({ label, before, after }: { label: string; before: string; after: string }) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 border-t border-border py-2 text-xs">
      <span className="text-muted">{label}</span>
      <span className="min-w-16 text-right font-mono tabular-nums">{before}</span>
      <span className="min-w-16 text-right font-mono tabular-nums">{after}</span>
    </div>
  );
}

function ServiceEvidence({ row, primary }: { row: ServiceComparison; primary: boolean }) {
  return (
    <article className={`rounded-xl border p-3 ${primary ? "border-accent/50 bg-accent/5" : "border-border bg-surface"}`} aria-label={`${row.name} comparison`}>
      <div className="flex items-center justify-between gap-2">
        <h3 className="truncate text-sm font-semibold">{row.name}</h3>
        {primary ? <span className="shrink-0 text-[10px] font-semibold text-accent">Release service</span> : null}
      </div>
      <div className="mt-2 grid grid-cols-[minmax(0,1fr)_auto_auto] gap-3 text-[10px] font-semibold uppercase tracking-wide text-muted">
        <span>Signal</span><span className="min-w-16 text-right">Before</span><span className="min-w-16 text-right">After</span>
      </div>
      <Metric label="Requests" before={count(row.before?.requests)} after={count(row.after?.requests)} />
      <Metric label="Errors" before={count(row.before?.errors)} after={count(row.after?.errors)} />
      <Metric label="Error rate" before={percent(row.errorRateBefore)} after={percent(row.errorRateAfter)} />
      <Metric label="p95 latency" before={latency(row.before?.p95_ms)} after={latency(row.after?.p95_ms)} />
      {(!row.before || !row.after || row.before.requests === 0 || row.after.requests === 0) ? (
        <p className="pt-2 text-[11px] text-muted">At least one window has no request sample for this service.</p>
      ) : null}
    </article>
  );
}

export function ChangesPage({ projectId, environmentId, initialEventId }: { projectId: string; environmentId: string; initialEventId: string | null }) {
  const session = useSession();
  const project = session.data?.projects.find((item) => item.id === projectId);
  const environment = project?.environments.find((item) => item.id === environmentId);
  const [rangeEnd, setRangeEnd] = useState<bigint>(nowNs);
  const [selectedId, setSelectedId] = useState<string | null>(initialEventId);
  const path = scopedPath(projectId, environmentId);
  const changes = useQuery({
    queryKey: ["changes", projectId, environmentId, rangeEnd.toString()],
    enabled: !!environment,
    placeholderData: keepPreviousData,
    queryFn: async () => unwrap(await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/events", {
      params: { path, query: {
        from: (rangeEnd - 7n * DAY_NS).toString(), to: rangeEnd.toString(),
        event_type: "deployment.finished", limit: 1000,
      } },
    })),
  });
  const releaseEvents = changes.data?.events ?? [];
  const selected: Event | null = releaseEvents.find((event) => event.id === selectedId) ?? (selectedId ? null : releaseEvents[0] ?? null);
  const eventTime = parseNs(selected?.time_ns);
  const window = eventTime !== null ? comparisonWindow(eventTime, rangeEnd) : null;
  const before = useQuery({
    queryKey: ["change-before", projectId, environmentId, window?.before.from.toString(), window?.before.to.toString()],
    enabled: !!environment && window !== null,
    queryFn: async (): Promise<Services> => {
      if (!window) throw new Error("No comparison window");
      return unwrap(await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/services", {
        params: { path, query: { from: window.before.from.toString(), to: window.before.to.toString() } },
      }));
    },
  });
  const after = useQuery({
    queryKey: ["change-after", projectId, environmentId, window?.after.from.toString(), window?.after.to.toString()],
    enabled: !!environment && window !== null,
    queryFn: async (): Promise<Services> => {
      if (!window) throw new Error("No comparison window");
      return unwrap(await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/services", {
        params: { path, query: { from: window.after.from.toString(), to: window.after.to.toString() } },
      }));
    },
  });
  const evidence = useMemo(() => selected && before.data && after.data
    ? compareServices(selected.service_id, before.data.services, after.data.services)
    : [], [selected, before.data, after.data]);
  const warnings = window && before.data && after.data
    ? comparisonWarnings(before.data.completeness, after.data.completeness, window)
    : [];
  const base = `/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}`;

  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  if (!environment) return <main className="p-8 text-sm">This environment does not exist or you do not have access to it.</main>;

  return (
    <AppShell projectId={projectId} environmentId={environmentId} active="changes">
      <main className="h-full overflow-auto p-4 md:p-6">
        <div className="mx-auto max-w-7xl space-y-5">
          <header className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h1 className="text-2xl font-semibold tracking-tight">Change Replay</h1>
              <p className="mt-1 max-w-3xl text-sm text-muted">Compare equal windows around a deployment. Differences are observations, not proof that the deployment caused them.</p>
            </div>
            <button type="button" className="btn btn-sm" onClick={() => setRangeEnd(nowNs())}>Refresh changes</button>
          </header>

          <div className="grid gap-4 lg:grid-cols-[280px_minmax(0,1fr)]">
            <section className="card min-w-0 overflow-hidden" aria-label="Deployment changes">
              <div className="border-b border-border px-4 py-3">
                <h2 className="text-sm font-semibold">Deployments · last 7 days</h2>
                <p className="mt-1 text-xs text-muted">Recorded deployment.finished events</p>
              </div>
              {changes.isPending ? <p className="p-4 text-sm text-muted">Loading deployments…</p> : null}
              {changes.error ? <p role="alert" className="p-4 text-sm text-error">Cannot load deployments: {changes.error.message}</p> : null}
              {!changes.isPending && !changes.error && releaseEvents.length === 0 ? (
                <p className="p-4 text-sm text-muted">No deployment events in the last 7 days. Send a deployment.finished event through the events ingest API after each release.</p>
              ) : null}
              <div role="list" aria-label="Recorded deployments" className="max-h-[70vh] overflow-auto p-2">
                {releaseEvents.map((event) => {
                  const at = parseNs(event.time_ns);
                  return <div role="listitem" key={event.id}>
                    <button type="button" onClick={() => setSelectedId(event.id)} aria-current={selected?.id === event.id ? "true" : undefined}
                      className={`my-0.5 w-full rounded-lg p-2 text-left hover:bg-surface-2 ${selected?.id === event.id ? "bg-accent/10 ring-1 ring-accent/40" : ""}`}>
                      <span className="block truncate text-sm font-medium">{event.service_name}</span>
                      <span className="block truncate text-xs text-muted">{version(event) ?? "Version not supplied"}</span>
                      <span className="block text-[11px] text-muted">{at === null ? "Unknown time" : formatDateTime(at)}</span>
                    </button>
                  </div>;
                })}
              </div>
              {changes.data?.completeness.truncated ? <p role="status" className="border-t border-border p-3 text-xs text-warn">The deployment list is truncated. Narrower search is needed to see older changes.</p> : null}
            </section>

            <section className="card min-w-0 p-4 md:p-5" aria-label="Change evidence">
              {selected ? <>
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <h2 className="text-lg font-semibold">{selected.service_name} deployment</h2>
                    <p className="mt-1 text-xs text-muted">{eventTime === null ? "Unknown time" : formatDateTime(eventTime)} · {version(selected) ?? "Version not supplied"}</p>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <Link className="btn btn-sm" href={base}><Icon name="map" className="h-3.5 w-3.5" />Open timeline</Link>
                    {can(project, "actions.view") ? <Link className="btn btn-sm" href={`${base}/actions`}><Icon name="shield" className="h-3.5 w-3.5" />Review actions</Link> : null}
                  </div>
                </div>
                {window ? <>
                  <div className="mt-4 rounded-lg border border-border bg-surface-2 p-3 text-xs">
                    <p>Before: {formatDateTime(window.before.from)} – {formatDateTime(window.before.to)}</p>
                    <p className="mt-1">After: {formatDateTime(window.after.from)} – {formatDateTime(window.after.to)}</p>
                    <p className="mt-2 text-muted">Both periods are the same length. Telemetry may be sampled, and services without spans cannot be assessed by request metrics.</p>
                  </div>
                  {before.isPending || after.isPending ? <p role="status" className="mt-4 text-sm text-muted">Comparing service telemetry…</p> : null}
                  {before.error || after.error ? <p role="alert" className="mt-4 text-sm text-error">Cannot compare this change: {(before.error ?? after.error)?.message}</p> : null}
                  {warnings.length > 0 ? <div role="status" className="mt-4 rounded-lg border border-warn/40 bg-warn/10 p-3 text-xs text-warn">
                    {warnings.map((warning) => <p key={warning}>{warning}</p>)}
                  </div> : null}
                  {before.data && after.data ? <>
                    {evidence.length > 0 ? <div className="mt-4 grid gap-3 xl:grid-cols-2">
                      {evidence.map((row) => <ServiceEvidence key={row.serviceId} row={row} primary={row.serviceId === selected.service_id} />)}
                    </div> : <p className="mt-4 text-sm text-muted">No service request telemetry was recorded in either window.</p>}
                    <p className="mt-4 text-xs text-muted">Data coverage: {after.data.completeness.sampling === "unknown" ? "sampling is unknown" : after.data.completeness.sampling}. Only instrumented components appear. Review the traces and timeline before choosing an action.</p>
                  </> : null}
                </> : <p role="status" className="mt-4 text-sm text-muted">This deployment has no elapsed post-release window yet. Refresh in a moment.</p>}
              </> : <p className="text-sm text-muted">{selectedId ? "The linked deployment is outside the last 7 days or unavailable in this environment." : "Choose a deployment to compare its telemetry."}</p>}
            </section>
          </div>
        </div>
      </main>
    </AppShell>
  );
}
