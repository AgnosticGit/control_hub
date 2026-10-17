"use client";

import { useEffect, useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AppShell } from "@/components/shell/AppShell";
import { api, errorMessage, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { can } from "@/lib/permissions";
import { useSession } from "@/lib/session";
import { installationStatus } from "./installation-status";

type Installation = Schemas["InstallationDto"];
const BASE = "/api/v1/projects/{project_id}/environments/{environment_id}/control/installations" as const;

export function ControlPage({ projectId, environmentId }: { projectId: string; environmentId: string }) {
  const session = useSession();
  const project = session.data?.projects.find((item) => item.id === projectId);
  const permitted = can(project, "project.manage");
  const qc = useQueryClient();
  const path = scopedPath(projectId, environmentId);
  const [name, setName] = useState("");
  const [code, setCode] = useState("");
  const [reportedId, setReportedId] = useState<Record<string, string>>({});
  const [nowMs, setNowMs] = useState(0);
  useEffect(() => {
    const tick = () => setNowMs(Date.now());
    tick();
    const timer = window.setInterval(tick, 5000);
    return () => window.clearInterval(timer);
  }, []);
  const installations = useQuery({
    queryKey: ["control-installations", projectId, environmentId], enabled: permitted, refetchInterval: 5000,
    queryFn: async () => unwrap(await api.GET(BASE, { params: { path } })),
  });
  const refresh = () => void qc.invalidateQueries({ queryKey: ["control-installations", projectId, environmentId] });
  const create = useMutation({
    mutationFn: async () => unwrap(await api.POST(BASE, { params: { path }, body: { name } })),
    onSuccess: (result) => { setCode(result.enrollment_code); setName(""); refresh(); },
  });
  const confirm = useMutation({
    mutationFn: async ({ id, containerId }: { id: string; containerId: string }) =>
      unwrap(await api.POST(`${BASE}/{id}/confirm`, { params: { path: { ...path, id } }, body: { container_id: containerId } })),
    onSuccess: refresh,
  });
  const revoke = useMutation({
    mutationFn: async (id: string) => unwrap(await api.POST(`${BASE}/{id}/revoke`, { params: { path: { ...path, id } } })),
    onSuccess: refresh,
  });
  const error = installations.error ?? create.error ?? confirm.error ?? revoke.error;

  return <AppShell projectId={projectId} environmentId={environmentId} active="control">
    <main className="h-full overflow-auto p-6">
      <div className="mx-auto max-w-4xl space-y-5">
        <h1 className="text-2xl font-semibold">Karbird Control</h1>
        <p className="text-sm text-muted">Install Control on a Docker host to start, stop, or restart one confirmed container. Collector stays separate and needs no Docker write access.</p>
        {!permitted ? <p className="card p-4 text-sm">Only a project manager can connect a Docker host.</p> : <>
          <form aria-label="Connect Docker host" className="card space-y-3 p-4" onSubmit={(event: FormEvent) => { event.preventDefault(); create.mutate(); }}>
            <h2 className="font-semibold">Connect one container</h2>
            <label className="block text-sm">Installation name
              <input className="input mt-1 block w-full" required maxLength={64} value={name} onChange={(event) => setName(event.target.value)} placeholder="production Docker host" />
            </label>
            <button type="submit" className="btn btn-primary" disabled={create.isPending}>Create one-time enrollment code</button>
          </form>
          {code && <section className="card space-y-2 p-4" aria-label="Enrollment code">
            <h2 className="font-semibold">One-time enrollment code</h2>
            <p className="text-sm text-muted">Save this code in a local secret file on the Docker host. It is shown once and expires in 15 minutes.</p>
            <code aria-label="One-time code" className="block break-all rounded bg-bg p-2">{code}</code>
            <p className="text-sm">Follow the versioned installation instructions in <code>deploy/control/README.md</code>. Control connects outbound; no inbound Docker port is needed.</p>
            <button type="button" className="btn" onClick={() => setCode("")}>Hide code</button>
          </section>}
          <section aria-label="Control installations" className="space-y-3">
            <h2 className="font-semibold">Installations</h2>
            {(installations.data ?? []).map((item: Installation) => <article key={item.id} aria-label={`Control ${item.name}`} className="card space-y-2 p-4">
              <div className="flex justify-between gap-2"><strong>{item.name}</strong><span>{installationStatus(item, nowMs)}</span></div>
              <p className="break-all text-xs text-muted">Installation: {item.id}</p>
              <p className="text-sm">Reported: {item.reported_container_name ?? "—"} · {item.reported_state ?? "not connected"}</p>
              {item.reported_container_id && <p className="break-all font-mono text-xs">Docker ID: {item.reported_container_id}</p>}
              {item.last_seen_at && <p className="text-xs text-muted">Last contact: {item.last_seen_at}</p>}
              {!item.revoked_at && !item.confirmed_at && item.reported_container_id && <div className="space-y-2">
                <label className="block text-sm">Confirm the full Docker ID shown above
                  <input className="input mt-1 block w-full font-mono text-xs" value={reportedId[item.id] ?? ""} onChange={(event) => setReportedId({ ...reportedId, [item.id]: event.target.value })} />
                </label>
                <button type="button" className="btn btn-primary" disabled={reportedId[item.id] !== item.reported_container_id || confirm.isPending}
                  onClick={() => confirm.mutate({ id: item.id, containerId: item.reported_container_id ?? "" })}>Allow control of this container</button>
              </div>}
              {!item.revoked_at && <button type="button" className="btn" disabled={revoke.isPending} onClick={() => revoke.mutate(item.id)}>Revoke installation</button>}
            </article>)}
            {installations.data?.length === 0 && <p className="text-sm text-muted">No Control installations yet.</p>}
          </section>
          {error && <p role="alert" className="text-sm text-error">{errorMessage(error)}</p>}
        </>}
      </div>
    </main>
  </AppShell>;
}
