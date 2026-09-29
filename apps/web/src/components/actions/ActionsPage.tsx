"use client";

import Link from "next/link";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { api, ApiRequestError, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { useSession } from "@/lib/session";

type Plan = Schemas["ActionPlanDto"];
type Spec = Schemas["ActionSpecDto"];

const BASE = "/api/v1/projects/{project_id}/environments/{environment_id}/actions" as const;
const OPEN_STATES = new Set(["proposed", "approved", "running"]);

function message(e: unknown) {
  return e instanceof ApiRequestError ? e.message : e instanceof Error ? e.message : String(e);
}

function when(iso: string | null | undefined) {
  return iso ? new Date(iso).toLocaleString() : "—";
}

function stateClass(state: string) {
  if (state === "succeeded") return "text-ok";
  if (state === "failed" || state === "verification_failed") return "text-error";
  if (OPEN_STATES.has(state)) return "text-warn";
  return "text-muted";
}

/** Plan fields come from the API and may echo telemetry-derived text: shown as text only. */
function Json({ value, label }: { value: unknown; label: string }) {
  return (
    <pre aria-label={label} className="mt-1 max-h-64 overflow-auto rounded bg-bg p-2 font-mono text-xs whitespace-pre-wrap">
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}

function ProposeForm({ specs, onPropose, pending }: { specs: Spec[]; onPropose: (b: Schemas["ProposeActionRequest"]) => void; pending: boolean }) {
  const allowed = specs.filter((s) => s.allowed_here);
  const [type, setType] = useState(allowed[0]?.action_type ?? "");
  const spec = allowed.find((s) => s.action_type === type);
  const [target, setTarget] = useState(spec?.targets[0] ?? "");
  const [params, setParams] = useState<Record<string, string>>({});
  const [reason, setReason] = useState("");

  if (allowed.length === 0) {
    return <p className="mt-6 text-sm text-muted">Policy allows no actions in this environment (demo-only actions need a demo environment).</p>;
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
    const values: Record<string, number> = {};
    for (const p of spec.params) values[p.name] = Number(params[p.name] ?? p.min);
    onPropose({ action_type: spec.action_type, target, params: values, reason });
  };

  return (
    <form onSubmit={onSubmit} aria-label="Propose action" className="mt-6 space-y-3 rounded border border-border bg-surface p-4 text-sm">
      <div className="flex flex-wrap items-end gap-3">
        <label className="flex flex-col gap-1">
          <span className="text-xs text-muted">Action</span>
          <select value={type} onChange={(e) => choose(e.target.value)} className="rounded border border-border bg-bg px-2 py-1">
            {allowed.map((s) => (
              <option key={s.action_type} value={s.action_type}>
                {s.title} ({s.risk} risk)
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-xs text-muted">Target</span>
          <select value={target} onChange={(e) => setTarget(e.target.value)} className="rounded border border-border bg-bg px-2 py-1">
            {(spec?.targets ?? []).map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </label>
        {(spec?.params ?? []).map((p) => (
          <label key={p.name} className="flex flex-col gap-1" title={p.description}>
            <span className="text-xs text-muted">
              {p.name} ({p.min}–{p.max})
            </span>
            <input
              type="number"
              required
              min={p.min}
              max={p.max}
              step={p.integer ? 1 : 0.01}
              value={params[p.name] ?? String(p.min)}
              onChange={(e) => setParams((cur) => ({ ...cur, [p.name]: e.target.value }))}
              className="w-28 rounded border border-border bg-bg px-2 py-1"
            />
          </label>
        ))}
      </div>
      {spec ? <p className="text-xs text-muted">{spec.description} Verification: {spec.verification}</p> : null}
      <label className="flex flex-col gap-1">
        <span className="text-xs text-muted">Reason (recorded in the audit log)</span>
        <input
          required
          minLength={5}
          maxLength={500}
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          className="rounded border border-border bg-bg px-2 py-1"
        />
      </label>
      <button type="submit" disabled={pending} className="rounded bg-accent px-3 py-1 font-medium text-bg disabled:opacity-50">
        Propose plan
      </button>
      <p className="text-xs text-muted">Every plan needs approval by an admin other than the proposer before the executor runs it.</p>
    </form>
  );
}

function PlanDetail({ projectId, environmentId, id }: { projectId: string; environmentId: string; id: string }) {
  const path = scopedPath(projectId, environmentId);
  const detail = useQuery({
    queryKey: ["action", projectId, environmentId, id],
    queryFn: async () => unwrap(await api.GET(`${BASE}/{plan_id}`, { params: { path: { ...path, plan_id: id } } })),
    refetchInterval: (q) => (q.state.data && OPEN_STATES.has(q.state.data.plan.state) ? 2000 : false),
  });
  if (detail.isPending) return <p className="text-sm text-muted">Loading…</p>;
  if (detail.error) return <p role="alert" className="text-sm text-error">{message(detail.error)}</p>;
  const { plan, audit } = detail.data;
  return (
    <section aria-label="Plan detail" className="space-y-3 rounded border border-border bg-surface p-4 text-sm">
      <h2 className="font-medium">
        {plan.title} · {plan.target} · <span className={stateClass(plan.state)}>{plan.state}</span>
      </h2>
      <p className="text-xs text-muted">
        Reason: {plan.reason}
        {plan.decision_comment ? ` · Decision comment: ${plan.decision_comment}` : ""}
      </p>
      <div>
        <h3 className="text-xs text-muted">Policy</h3>
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
          <h3 className="text-xs text-muted">Result</h3>
          <Json value={plan.result} label="Result" />
        </div>
      ) : null}
      {plan.verification ? (
        <div>
          <h3 className="text-xs text-muted">Verification</h3>
          <Json value={plan.verification} label="Verification" />
        </div>
      ) : null}
      <div>
        <h3 className="text-xs text-muted">Audit trail</h3>
        <ol aria-label="Audit trail" className="mt-1 space-y-0.5 text-xs">
          {audit.map((a) => (
            <li key={a.id}>
              {when(a.occurred_at)} · {a.action} · {a.actor_kind} · {a.outcome}
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
  const qc = useQueryClient();
  const path = scopedPath(projectId, environmentId);
  const [selected, setSelected] = useState<string | null>(null);

  const project = session.data?.projects.find((p) => p.id === projectId);
  const env = project?.environments.find((e) => e.id === environmentId);
  const role = project?.role;
  const isAdmin = role === "admin" || role === "owner";
  const canPropose = role !== undefined && role !== "viewer";
  const me = session.data?.user.email;

  const catalog = useQuery({
    queryKey: ["action-catalog", projectId, environmentId],
    enabled: env !== undefined,
    queryFn: async () => unwrap(await api.GET(`${BASE}/catalog`, { params: { path } })),
  });
  const plans = useQuery({
    queryKey: ["actions", projectId, environmentId],
    enabled: env !== undefined,
    queryFn: async () => unwrap(await api.GET(BASE, { params: { path } })),
    refetchInterval: (q) => ((q.state.data ?? []).some((p) => OPEN_STATES.has(p.state)) ? 2000 : 15_000),
  });
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ["actions", projectId, environmentId] });
    void qc.invalidateQueries({ queryKey: ["action", projectId, environmentId] });
  };

  const propose = useMutation({
    mutationFn: async (body: Schemas["ProposeActionRequest"]) => unwrap(await api.POST(BASE, { params: { path }, body })),
    onSuccess: (p) => {
      setSelected(p.id);
      refresh();
    },
  });
  const decide = useMutation({
    mutationFn: async ({ plan, verb, comment }: { plan: Plan; verb: "approve" | "reject" | "cancel"; comment: string | null }) =>
      unwrap(await api.POST(`${BASE}/{plan_id}/${verb}`, { params: { path: { ...path, plan_id: plan.id } }, body: { comment } })),
    onSuccess: () => refresh(),
  });

  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  if (!env) return <p className="p-8 text-sm">This environment does not exist or you do not have access to it.</p>;

  const error = propose.error ?? decide.error ?? plans.error ?? catalog.error;

  return (
    <main className="mx-auto max-w-6xl p-8">
      <Link href={`/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}`} className="text-sm text-accent">
        ← Back to workspace
      </Link>
      <h1 className="mt-4 text-xl font-semibold">
        Actions · {project?.name} / {env.name}
      </h1>
      <p className="mt-2 text-sm text-muted">
        Controlled changes to the observed system. Only allowlisted actions exist; each plan is checked by policy, needs
        approval by a different admin, runs in the separate executor, is verified from telemetry and is fully audited.
      </p>

      {canPropose && catalog.data ? (
        <ProposeForm specs={catalog.data.actions} onPropose={(b) => propose.mutate(b)} pending={propose.isPending} />
      ) : (
        <p className="mt-6 text-sm text-muted">{canPropose ? "Loading catalog…" : "Proposing actions requires the operator role."}</p>
      )}

      {error ? (
        <p role="alert" className="mt-4 text-sm text-error">
          {message(error)}
        </p>
      ) : null}

      <div className="mt-6 grid gap-6 lg:grid-cols-[3fr_2fr]">
        <table className="w-full self-start text-left text-sm" aria-label="Action plans">
          <thead className="text-xs text-muted">
            <tr>
              <th className="py-2">Action</th>
              <th>Target</th>
              <th>State</th>
              <th>Proposed by</th>
              <th>Created</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {(plans.data ?? []).map((p) => {
              const mine = p.created_by === me;
              return (
                <tr
                  key={p.id}
                  className={`cursor-pointer border-t border-border ${selected === p.id ? "bg-surface" : ""}`}
                  onClick={() => setSelected(p.id)}
                >
                  <td className="py-2">{p.title}</td>
                  <td>{p.target}</td>
                  <td className={stateClass(p.state)}>{p.state}</td>
                  <td>{p.created_by}</td>
                  <td>{when(p.created_at)}</td>
                  <td className="space-x-1 text-right" onClick={(e) => e.stopPropagation()}>
                    {p.state === "proposed" && isAdmin && !mine ? (
                      <>
                        <button
                          type="button"
                          className="rounded border border-ok/60 px-2 py-0.5 text-xs text-ok hover:bg-ok/10"
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
                          className="rounded border border-error/60 px-2 py-0.5 text-xs text-error hover:bg-error/10"
                          onClick={() => {
                            const comment = window.prompt("Reason for rejecting:");
                            if (comment && comment.trim()) decide.mutate({ plan: p, verb: "reject", comment });
                          }}
                        >
                          Reject
                        </button>
                      </>
                    ) : null}
                    {(p.state === "proposed" || p.state === "approved") && (mine || isAdmin) ? (
                      <button
                        type="button"
                        className="rounded border border-border px-2 py-0.5 text-xs hover:border-accent"
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
        <div>
          {selected ? (
            <PlanDetail projectId={projectId} environmentId={environmentId} id={selected} />
          ) : (
            <p className="text-sm text-muted">Select a plan to see its policy checks, result, verification and audit trail.</p>
          )}
        </div>
      </div>
    </main>
  );
}
