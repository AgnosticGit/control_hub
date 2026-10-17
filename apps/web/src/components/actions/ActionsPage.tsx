"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { canCancelPlan, canDecidePlan, canStartAction } from "@/lib/permissions";
import { useSession } from "@/lib/session";
import { formatIsoDateTime } from "@/lib/time";
import { AppShell } from "@/components/shell/AppShell";
import { Icon } from "@/components/ui/Icon";
import { ErrorNotice } from "@/components/ui/ErrorNotice";
import { paramValues } from "./params";
import { stateClass } from "./controls";
import { ParamFields } from "./ParamFields";
import { ACTIONS_BASE as BASE, errorMessage as message, useActions } from "./api";
import { parseActionFrame, upsertActionPlan } from "./live";

type Spec = Schemas["ActionSpecDto"];

/** Plan fields come from the API and may echo telemetry-derived text: shown as text only. */
function Json({ value, label }: { value: unknown; label: string }) {
  return (
    <pre aria-label={label} className="mt-1 max-h-64 overflow-auto rounded-lg border border-border bg-bg/70 p-3 font-mono text-xs whitespace-pre-wrap">
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}

function ProposeForm({
  specs,
  canRun,
  onPropose,
  pending,
}: {
  specs: Spec[];
  canRun: boolean;
  onPropose: (b: Schemas["ProposeActionRequest"]) => void;
  pending: boolean;
}) {
  const allowed = specs.filter((s) => s.allowed_here && s.targets.length > 0);
  const [type, setType] = useState(allowed[0]?.action_type ?? "");
  const spec = allowed.find((s) => s.action_type === type);
  const [target, setTarget] = useState(spec?.targets[0] ?? "");
  const [params, setParams] = useState<Record<string, string>>({});
  const [reason, setReason] = useState("");

  if (allowed.length === 0) {
    return <p className="card p-4 text-sm text-muted">Policy allows no actions in this environment (demo-only actions need a demo environment).</p>;
  }
  const choose = (t: string) => {
    const s = allowed.find((x) => x.action_type === t);
    setType(t);
    setTarget(s?.targets[0] ?? "");
    setParams({});
  };
  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!spec) return;
    onPropose({ action_type: spec.action_type, target, params: paramValues(spec.params, params), reason });
  };

  return (
    <form onSubmit={onSubmit} aria-label={canRun ? "Run action" : "Propose action"} className="card space-y-4 p-5 text-sm">
      <div className="flex flex-wrap items-end gap-3">
        <label className="flex flex-col gap-1.5">
          <span className="label">Action</span>
          <select value={type} onChange={(e) => choose(e.target.value)} className="select h-9 min-w-64 text-sm">
            {allowed.map((s) => (
              <option key={s.action_type} value={s.action_type}>
                {s.title} ({s.risk} risk)
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1.5">
          <span className="label">Target</span>
          <select value={target} onChange={(e) => setTarget(e.target.value)} className="select h-9 min-w-40 text-sm">
            {(spec?.targets ?? []).map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </label>
        <ParamFields
          params={spec?.params ?? []}
          values={params}
          onChange={(name, value) => setParams((cur) => ({ ...cur, [name]: value }))}
        />
      </div>
      {spec ? <p className="text-xs text-muted">{spec.description} Verification: {spec.verification}</p> : null}
      <label className="flex flex-col gap-1.5">
        <span className="label">Reason (recorded in the audit log)</span>
        <input
          required
          minLength={5}
          maxLength={500}
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          className="input h-9"
        />
      </label>
      <button type="submit" disabled={pending} className="btn btn-primary h-9 px-4">
        <Icon name={canRun ? "bolt" : "check"} className="h-4 w-4" />
        {canRun ? "Run" : "Propose plan"}
      </button>
      <p className="text-xs text-muted">
        {canRun
          ? "Your role may run actions: the executor starts it right away."
          : "Your role may only propose: someone allowed to run actions has to approve the plan first."}
      </p>
    </form>
  );
}

function PlanDetail({ projectId, environmentId, id }: { projectId: string; environmentId: string; id: string }) {
  const path = scopedPath(projectId, environmentId);
  const detail = useQuery({
    queryKey: ["action", projectId, environmentId, id],
    queryFn: async () => unwrap(await api.GET(`${BASE}/{plan_id}`, { params: { path: { ...path, plan_id: id } } })),
    refetchInterval: false,
  });
  if (detail.isPending) return <p className="text-sm text-muted">Loading…</p>;
  if (detail.error) return <p role="alert" className="text-sm text-error">{message(detail.error)}</p>;
  const { plan, audit } = detail.data;
  return (
    <section aria-label="Plan detail" className="card space-y-4 p-5 text-sm">
      <h2 className="text-base font-semibold">
        {plan.title} · {plan.target} · <span className={stateClass(plan.state)}>{plan.state}</span>
      </h2>
      <p className="text-xs text-muted">
        Reason: {plan.reason}
        {plan.decision_comment ? ` · Decision comment: ${plan.decision_comment}` : ""}
      </p>
      <div>
        <h3 className="label">Policy</h3>
        <ul className="mt-1 space-y-0.5">
          {plan.policy.checks.map((c) => (
            <li key={c.name} className={c.passed ? "text-ok" : "text-error"}>
              {c.passed ? "✓" : "✗"} {c.name}: {c.detail}
            </li>
          ))}
        </ul>
      </div>
      {plan.result ? (
        <div>
          <h3 className="label">Result</h3>
          <Json value={plan.result} label="Result" />
        </div>
      ) : null}
      {plan.verification ? (
        <div>
          <h3 className="label">Verification</h3>
          <Json value={plan.verification} label="Verification" />
        </div>
      ) : null}
      <div>
        <h3 className="label">Audit trail</h3>
        <ol aria-label="Audit trail" className="mt-1 space-y-0.5 text-xs">
          {audit.map((a) => (
            <li key={a.id}>
              {formatIsoDateTime(a.occurred_at)} · {a.action} · {a.actor_kind} · {a.outcome}
              {a.reason ? ` · ${a.reason}` : ""}
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}

export function ActionsPage({ projectId, environmentId }: { projectId: string; environmentId: string }) {
  const session = useSession();
  const queryClient = useQueryClient();
  const [selected, setSelected] = useState<string | null>(null);

  const project = session.data?.projects.find((p) => p.id === projectId);
  const env = project?.environments.find((e) => e.id === environmentId);
  const canPropose = canStartAction(project);
  const me = session.data?.user.email;
  const { catalog, plans, propose, decide } = useActions(projectId, environmentId, env !== undefined);

  useEffect(() => {
    if (!env) return;
    let socket: WebSocket | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let attempt = 0;
    let closed = false;
    const connect = () => {
      if (closed) return;
      const url = new URL(`/api/v1/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(environmentId)}/live`, window.location.origin);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      socket = new WebSocket(url);
      socket.onopen = () => { attempt = 0; };
      socket.onmessage = (event) => {
        if (typeof event.data !== "string") return;
        const plan = parseActionFrame(event.data);
        if (!plan) return;
        queryClient.setQueryData<Schemas["ActionPlanDto"][]>(["actions", projectId, environmentId], (current) => upsertActionPlan(current, plan));
        void queryClient.invalidateQueries({ queryKey: ["action", projectId, environmentId, plan.id] });
      };
      socket.onclose = () => {
        if (closed) return;
        attempt += 1;
        timer = setTimeout(connect, Math.min(30_000, 500 * 2 ** Math.min(attempt - 1, 6)));
      };
      socket.onerror = () => socket?.close();
    };
    connect();
    return () => {
      closed = true;
      if (timer) clearTimeout(timer);
      socket?.close(1000, "actions page unmounted");
    };
  }, [environmentId, env, projectId, queryClient]);

  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  if (!env) {
    return (
      <main className="flex min-h-screen items-center justify-center p-8">
        <div className="card max-w-md p-6 text-sm">
          <p>This environment does not exist or you do not have access to it.</p>
          <Link href="/app" className="btn mt-4">
            <Icon name="arrowLeft" className="h-3.5 w-3.5" />
            Back to environments
          </Link>
        </div>
      </main>
    );
  }

  const error = propose.error ?? decide.error ?? plans.error ?? catalog.error;

  return (
    <AppShell
      projectId={projectId}
      environmentId={environmentId}
      active="actions"
    >
      <main className="h-full overflow-auto">
        <div className="mx-auto max-w-7xl space-y-5 p-6">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">Actions</h1>
            <Link href={`/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}/control`} className="text-sm text-accent hover:underline">Karbird Control setup →</Link>
            <p className="mt-1 max-w-3xl text-sm text-muted">
              Controlled changes to the observed system. Only allowlisted actions exist; each plan is checked by policy, runs
              in the separate executor once someone allowed to run actions starts or approves it, is verified from telemetry
              and is fully audited. Services and containers can also be controlled from the System Map.
            </p>
          </div>

          {canPropose && catalog.data ? (
            <ProposeForm
              specs={catalog.data.actions}
              canRun={catalog.data.can_run}
              onPropose={(b) => propose.mutate(b, { onSuccess: (p) => setSelected(p.id) })}
              pending={propose.isPending}
            />
          ) : (
            <p className="card p-4 text-sm text-muted">{canPropose ? "Loading catalog…" : "Your role in this project does not include proposing actions."}</p>
          )}

          {error ? <ErrorNotice className="text-sm">{message(error)}</ErrorNotice> : null}

          <div className="grid gap-5 lg:grid-cols-[3fr_2fr]">
            <div className="card self-start overflow-hidden">
              <div className="card-header">
                <span className="card-title-icon">
                  <Icon name="list" className="h-3.5 w-3.5" />
                </span>
                Plans
              </div>
              <table className="w-full text-left text-sm" aria-label="Action plans">
                <thead className="table-head">
                  <tr>
                    <th className="px-4 py-2">Action</th>
                    <th className="px-2">Target</th>
                    <th className="px-2">State</th>
                    <th className="px-2">Proposed by</th>
                    <th className="px-2">Created</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
            {(plans.data ?? []).map((p) => {
              const mine = p.created_by === me;
              return (
                <tr
                  key={p.id}
                  className={`cursor-pointer border-t border-border transition-colors hover:bg-surface-2 ${selected === p.id ? "bg-accent/10" : ""}`}
                  onClick={() => setSelected(p.id)}
                >
                  <td className="px-4 py-2.5 font-medium">{p.title}</td>
                  <td className="px-2 font-mono text-xs">{p.target}</td>
                  <td className="px-2">
                    <span className={`badge border-current/40 ${stateClass(p.state)}`}>{p.state}</span>
                  </td>
                  <td className="px-2 text-xs text-muted">{p.created_by}</td>
                  <td className="px-2 text-xs text-muted">{formatIsoDateTime(p.created_at)}</td>
                  <td className="space-x-1 px-3 text-right whitespace-nowrap" onClick={(e) => e.stopPropagation()}>
                    {p.state === "proposed" && canDecidePlan(project) ? (
                      <>
                        <button
                          type="button"
                          className="btn btn-sm btn-ok"
                          onClick={() => {
                            if (window.confirm(`Approve “${p.title}” on ${p.target}? The executor will run it.`)) {
                              decide.mutate({ plan: p, verb: "approve", comment: null });
                            }
                          }}
                        >
                          Approve
                        </button>
                        <button
                          type="button"
                          className="btn btn-sm btn-danger"
                          onClick={() => {
                            const comment = window.prompt("Reason for rejecting:");
                            if (comment && comment.trim()) decide.mutate({ plan: p, verb: "reject", comment });
                          }}
                        >
                          Reject
                        </button>
                      </>
                    ) : null}
                    {(p.state === "proposed" || p.state === "approved") && canCancelPlan(project, mine) ? (
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => {
                          if (window.confirm(`Cancel “${p.title}”?`)) decide.mutate({ plan: p, verb: "cancel", comment: null });
                        }}
                      >
                        Cancel
                      </button>
                    ) : null}
                  </td>
                </tr>
              );
            })}
                </tbody>
              </table>
              {(plans.data ?? []).length === 0 && !plans.isPending ? (
                <p className="border-t border-border px-4 py-6 text-center text-sm text-muted">No plans yet.</p>
              ) : null}
            </div>
            <div>
              {selected ? (
                <PlanDetail projectId={projectId} environmentId={environmentId} id={selected} />
              ) : (
                <p className="card p-4 text-sm text-muted">Select a plan to see its policy checks, result, verification and audit trail.</p>
              )}
            </div>
          </div>
        </div>
      </main>
    </AppShell>
  );
}
