"use client";

import Link from "next/link";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { AppShell } from "@/components/shell/AppShell";
import { ErrorNotice } from "@/components/ui/ErrorNotice";
import { api, apiSentence, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { can } from "@/lib/permissions";
import { useSession } from "@/lib/session";
import { formatIsoDateTime, nowNs, NS_PER_SEC } from "@/lib/time";
import { incidentMomentHref, serviceLabel } from "./model";

type Rule = Schemas["IncidentRuleDto"];
type Incident = Schemas["IncidentDto"];
type View = "open" | "history" | "rules";

const RULES = "/api/v1/projects/{project_id}/environments/{environment_id}/incident-rules" as const;
const INCIDENTS = "/api/v1/projects/{project_id}/environments/{environment_id}/incidents" as const;
const SERVICES = "/api/v1/projects/{project_id}/environments/{environment_id}/services" as const;

function metricTitle(kind: string): string {
  return kind === "error_rate" ? "Error rate" : "p95 latency";
}

function ruleLabel(rule: Pick<Rule, "kind" | "threshold" | "min_requests">): string {
  return `${metricTitle(rule.kind)} > ${rule.threshold}${rule.kind === "error_rate" ? "%" : " ms"} · at least ${rule.min_requests} requests / 5 min`;
}

function evidence(incident: Incident): string {
  const ratio = incident.requests > 0 ? ` · ${(incident.errors * 100 / incident.requests).toFixed(1)}% errors` : "";
  const p95 = incident.p95_ms === null || incident.p95_ms === undefined ? "" : ` · p95 ${incident.p95_ms.toFixed(0)} ms`;
  return `${incident.requests} requests${ratio}${p95}`;
}

export function IncidentsPage({ projectId, environmentId }: { projectId: string; environmentId: string }) {
  const session = useSession();
  const project = session.data?.projects.find((item) => item.id === projectId);
  const manage = can(project, "incidents.manage");
  const path = scopedPath(projectId, environmentId);
  const qc = useQueryClient();
  const [view, setView] = useState<View>("open");
  const [serviceId, setServiceId] = useState("");
  const [kind, setKind] = useState("error_rate");
  const [threshold, setThreshold] = useState("5");
  const [minRequests, setMinRequests] = useState("20");
  const [editingRuleId, setEditingRuleId] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);

  const rules = useQuery({
    queryKey: ["incident-rules", projectId, environmentId],
    queryFn: async () => unwrap(await api.GET(RULES, { params: { path } })),
    refetchInterval: 15_000,
  });
  const incidents = useQuery({
    queryKey: ["incidents", projectId, environmentId],
    queryFn: async () => unwrap(await api.GET(INCIDENTS, { params: { path } })),
    refetchInterval: 15_000,
  });
  const services = useQuery({
    queryKey: ["incident-services", projectId, environmentId],
    queryFn: async () => {
      const to = nowNs();
      return unwrap(await api.GET(SERVICES, { params: { path, query: { from: (to - 3600n * NS_PER_SEC).toString(), to: to.toString() } } }));
    },
    staleTime: 60_000,
  });
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ["incident-rules", projectId, environmentId] });
    void qc.invalidateQueries({ queryKey: ["incidents", projectId, environmentId] });
  };
  const create = useMutation({
    mutationFn: async (body: Schemas["RuleRequest"]) => unwrap(await api.POST(RULES, { params: { path }, body })),
    onSuccess: () => { refresh(); resetForm(); },
  });
  const update = useMutation({
    mutationFn: async ({ id, body }: { id: string; body: Schemas["UpdateRuleRequest"] }) =>
      unwrap(await api.PATCH(`${RULES}/{rule_id}`, { params: { path: { ...path, rule_id: id } }, body })),
    onSuccess: () => { refresh(); resetForm(); },
  });
  const acknowledge = useMutation({
    mutationFn: async (id: string) =>
      unwrap(await api.POST(`${INCIDENTS}/{incident_id}/acknowledge`, { params: { path: { ...path, incident_id: id } } })),
    onSuccess: refresh,
  });

  function resetForm() {
    setServiceId("");
    setKind("error_rate");
    setThreshold("5");
    setMinRequests("20");
    setEditingRuleId(null);
    setFormError(null);
  }

  function editRule(rule: Rule) {
    setEditingRuleId(rule.id);
    setServiceId(rule.service_id);
    setKind(rule.kind);
    setThreshold(String(rule.threshold));
    setMinRequests(String(rule.min_requests));
    setFormError(null);
  }

  function submitRule(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const thresholdValue = Number(threshold);
    const min = Number(minRequests);
    if (!serviceId.trim() || !Number.isFinite(thresholdValue) || thresholdValue <= 0 || !Number.isInteger(min) || min < 1) {
      setFormError("Enter a service, a positive threshold and at least one request.");
      return;
    }
    setFormError(null);
    if (editingRuleId) {
      const current = rules.data?.find((rule) => rule.id === editingRuleId);
      if (current) update.mutate({ id: current.id, body: { threshold: thresholdValue, min_requests: min, enabled: current.enabled } });
    } else {
      create.mutate({ service_id: serviceId.trim(), kind, threshold: thresholdValue, min_requests: min });
    }
  }

  const open = (incidents.data ?? []).filter((item) => item.state === "open");
  const shown = view === "open" ? open : (incidents.data ?? []).filter((item) => item.state !== "open");
  const mutationError = create.error ?? update.error ?? acknowledge.error;

  return <AppShell projectId={projectId} environmentId={environmentId} active="incidents">
    <main className="h-full overflow-y-auto bg-bg px-5 py-6 text-text md:px-8" aria-label="Incidents">
      <div className="mx-auto max-w-6xl space-y-6">
        <header className="flex flex-wrap items-end justify-between gap-4">
          <div>
            <p className="label">Operations</p>
            <h1 className="mt-1 text-2xl font-semibold">Incident queue</h1>
            <p className="mt-2 max-w-2xl text-sm text-muted">Rules evaluate closed five-minute request windows. Two consecutive breaches open an incident; two healthy windows recover it. Low traffic stays unknown.</p>
          </div>
          <span className="rounded-full border border-border bg-surface px-3 py-1.5 text-sm" aria-label="Open incidents count">{open.length} open</span>
        </header>

        <div role="tablist" aria-label="Incident views" className="flex gap-2 border-b border-border pb-2">
          {(["open", "history", "rules"] as const).map((item) => <button key={item} type="button" role="tab" aria-selected={view === item} onClick={() => setView(item)} className={`rounded-lg px-3 py-2 text-sm capitalize ${view === item ? "bg-accent/15 text-accent" : "text-muted hover:bg-surface-2 hover:text-text"}`}>{item === "open" ? `Open (${open.length})` : item}</button>)}
        </div>
        {incidents.isError || rules.isError ? <ErrorNotice>{apiSentence(incidents.error ?? rules.error)}</ErrorNotice> : null}
        {mutationError ? <ErrorNotice>{apiSentence(mutationError)}</ErrorNotice> : null}

        {view !== "rules" ? <section aria-label={view === "open" ? "Open incidents" : "Incident history"} className="space-y-3">
          {incidents.isPending ? <p className="text-sm text-muted">Loading incidents…</p> : null}
          {!incidents.isPending && shown.length === 0 ? <div className="rounded-xl border border-border bg-surface p-6 text-sm text-muted">{view === "open" ? "No open incidents. Create a rule to start monitoring service requests." : "No recovered or closed incidents yet."}</div> : null}
          {shown.map((incident) => <article key={incident.id} className="rounded-xl border border-border bg-surface p-5">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <div className="flex flex-wrap items-center gap-2"><h2 className="font-semibold">{serviceLabel(incident.service_id, services.data?.services)}</h2><span className={`rounded-full px-2 py-0.5 text-xs ${incident.state === "open" ? "bg-error/15 text-error" : "bg-surface-2 text-muted"}`}>{incident.state}</span>{incident.acknowledged ? <span className="text-xs text-muted">Acknowledged</span> : null}</div>
                <p className="mt-1 text-sm text-muted">{metricTitle(incident.kind)} &gt; {incident.threshold}{incident.kind === "error_rate" ? "%" : " ms"} · opened {formatIsoDateTime(incident.created_at)}</p>
                <p className="mt-2 text-sm">{evidence(incident)}</p>
                {incident.closed_reason ? <p className="mt-1 text-xs text-muted">Closed: {incident.closed_reason.replaceAll("_", " ")}</p> : null}
              </div>
              <div className="flex items-center gap-2">
                <Link className="btn btn-sm" href={incidentMomentHref(projectId, environmentId, incident.opened_window_end_ns, incident.service_id)}>Open moment</Link>
                {manage && incident.state === "open" && !incident.acknowledged ? <button type="button" className="btn btn-sm btn-primary" disabled={acknowledge.isPending} onClick={() => acknowledge.mutate(incident.id)}>Acknowledge</button> : null}
              </div>
            </div>
          </article>)}
        </section> : <section aria-label="Incident rules" className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(300px,380px)]">
          <div className="space-y-3">
            <h2 className="text-lg font-semibold">Rules</h2>
            {rules.isPending ? <p className="text-sm text-muted">Loading rules…</p> : null}
            {!rules.isPending && rules.data?.length === 0 ? <p className="rounded-xl border border-border bg-surface p-5 text-sm text-muted">No rules yet.</p> : null}
            {rules.data?.map((rule) => <article key={rule.id} className="rounded-xl border border-border bg-surface p-4">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div><h3 className="font-medium">{serviceLabel(rule.service_id, services.data?.services)}</h3><p className="mt-1 text-sm text-muted">{ruleLabel(rule)}</p><p className="mt-2 text-xs text-muted">{rule.enabled ? `Observation: ${rule.observation_state}` : "Paused"}</p></div>
                {manage ? <div className="flex gap-2"><button type="button" className="btn btn-sm" onClick={() => editRule(rule)}>Edit</button><button type="button" className="btn btn-sm" disabled={update.isPending} onClick={() => update.mutate({ id: rule.id, body: { threshold: rule.threshold, min_requests: rule.min_requests, enabled: !rule.enabled } })}>{rule.enabled ? "Pause" : "Enable"}</button></div> : null}
              </div>
            </article>)}
          </div>
          {manage ? <form onSubmit={submitRule} aria-label={editingRuleId ? "Edit incident rule" : "Create incident rule"} className="h-fit space-y-4 rounded-xl border border-border bg-surface p-5">
            <h2 className="text-lg font-semibold">{editingRuleId ? "Edit rule" : "New rule"}</h2>
            <label className="flex flex-col gap-1.5 text-sm"><span>Service ID</span><input className="input" list="incident-service-options" required maxLength={200} disabled={editingRuleId !== null} value={serviceId} onChange={(event) => setServiceId(event.target.value)} /></label>
            <datalist id="incident-service-options">{services.data?.services.map((service) => <option key={service.service_id} value={service.service_id} label={service.name} />)}</datalist>
            <label className="flex flex-col gap-1.5 text-sm"><span>Metric</span><select className="select" disabled={editingRuleId !== null} value={kind} onChange={(event) => { setKind(event.target.value); setThreshold(event.target.value === "error_rate" ? "5" : "500"); }}><option value="error_rate">Error rate</option><option value="p95_latency">p95 latency</option></select></label>
            <label className="flex flex-col gap-1.5 text-sm"><span>Threshold ({kind === "error_rate" ? "%" : "ms"})</span><input className="input" type="number" required min="0.01" max={kind === "error_rate" ? "100" : "60000"} step="0.01" value={threshold} onChange={(event) => setThreshold(event.target.value)} /></label>
            <label className="flex flex-col gap-1.5 text-sm"><span>Minimum requests per 5 min</span><input className="input" type="number" required min="1" max="100000" step="1" value={minRequests} onChange={(event) => setMinRequests(event.target.value)} /></label>
            <p className="text-xs text-muted">Evaluation starts with the next closed minute. One rule per service and metric.</p>
            {formError ? <ErrorNotice>{formError}</ErrorNotice> : null}
            <div className="flex gap-2"><button type="submit" className="btn btn-primary" disabled={create.isPending || update.isPending}>{editingRuleId ? "Save rule" : "Create rule"}</button>{editingRuleId ? <button type="button" className="btn" onClick={resetForm}>Cancel</button> : null}</div>
          </form> : null}
        </section>}
      </div>
    </main>
  </AppShell>;
}
