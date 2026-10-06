"use client";

import Link from "next/link";
import { useState, type FormEvent } from "react";
import { canCancelPlan, canDecidePlan, canStartAction, can } from "@/lib/permissions";
import { useSession } from "@/lib/session";
import { Icon } from "@/components/ui/Icon";
import { errorMessage, useActions } from "./api";
import { controlsFor, plansFor, stateClass, type Control, type ControlSubject } from "./controls";
import { paramValues } from "./params";
import { ParamFields } from "./ParamFields";

interface Props {
  projectId: string;
  environmentId: string;
  subject: ControlSubject;
  /** Name shown in labels, e.g. the service or container name. */
  label: string;
}

/**
 * Controlled actions for one map card or container, in the right panel. Users who may run
 * actions start them right away; users who may only propose send a plan for approval.
 */
export function ActionControls({ projectId, environmentId, subject, label }: Props) {
  const session = useSession();
  const project = session.data?.projects.find((p) => p.id === projectId);
  const visible = can(project, "actions.view");
  const { catalog, plans, propose, decide } = useActions(projectId, environmentId, visible);
  const [open, setOpen] = useState<string | null>(null);

  if (!visible || !catalog.data) return null;
  const controls = controlsFor(catalog.data.actions, subject);
  if (controls.length === 0) return null;
  const canRun = catalog.data.can_run;
  const canStart = canStartAction(project);
  const me = session.data?.user.email;
  const recent = plansFor(plans.data ?? [], controls);
  const error = propose.error ?? decide.error;
  const base = `/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}`;

  return (
    <section aria-label={`Control ${label}`} className="space-y-2">
      <div className="flex items-center gap-2">
        <h3 className="label">Control</h3>
        <Link href={`${base}/actions`} className="ml-auto text-[11px] text-muted hover:text-accent">
          All actions →
        </Link>
      </div>
      {canStart ? (
        <ul className="space-y-1.5">
          {controls.map((c) => (
            <li key={c.spec.action_type} className="rounded-lg border border-border bg-bg/50">
              <button
                type="button"
                aria-expanded={open === c.spec.action_type}
                onClick={() => setOpen((cur) => (cur === c.spec.action_type ? null : c.spec.action_type))}
                className="flex w-full items-center gap-2 px-2.5 py-2 text-left text-xs font-medium hover:text-accent"
              >
                <Icon name="bolt" className="h-3.5 w-3.5 text-accent" />
                {c.spec.title}
                <span className="ml-auto text-[10px] font-normal text-muted">{c.spec.risk} risk</span>
              </button>
              {open === c.spec.action_type ? (
                <ControlForm
                  key={c.spec.action_type}
                  control={c}
                  canRun={canRun}
                  pending={propose.isPending}
                  onSubmit={(body) => propose.mutate(body, { onSuccess: () => setOpen(null) })}
                />
              ) : null}
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-xs text-muted">Your role in this project does not include running or proposing actions.</p>
      )}
      {error ? (
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-2.5 py-1.5 text-xs text-error">
          {errorMessage(error)}
        </p>
      ) : null}
      {recent.length > 0 ? (
        <ol aria-label={`Recent actions on ${label}`} className="space-y-1 text-xs">
          {recent.map((p) => {
            const mine = p.created_by === me;
            return (
              <li key={p.id} className="flex flex-wrap items-center gap-1.5 rounded-lg border border-border bg-bg/50 px-2.5 py-1.5">
                <span className="font-medium">{p.title}</span>
                <span className="font-mono text-[10px] text-muted">{p.target}</span>
                <span className={`badge ml-auto border-current/40 ${stateClass(p.state)}`}>{p.state}</span>
                {p.state === "proposed" && canDecidePlan(project) ? (
                  <button
                    type="button"
                    className="btn btn-sm btn-ok"
                    aria-label={`Approve ${p.title} on ${p.target}`}
                    onClick={() => decide.mutate({ plan: p, verb: "approve", comment: null })}
                  >
                    Approve
                  </button>
                ) : null}
                {(p.state === "proposed" || p.state === "approved") && canCancelPlan(project, mine) ? (
                  <button
                    type="button"
                    className="btn btn-sm"
                    aria-label={`Cancel ${p.title} on ${p.target}`}
                    onClick={() => decide.mutate({ plan: p, verb: "cancel", comment: null })}
                  >
                    Cancel
                  </button>
                ) : null}
              </li>
            );
          })}
        </ol>
      ) : null}
    </section>
  );
}

function ControlForm({
  control,
  canRun,
  pending,
  onSubmit,
}: {
  control: Control;
  canRun: boolean;
  pending: boolean;
  onSubmit: (body: { action_type: string; target: string; params: Record<string, number>; reason: string }) => void;
}) {
  const { spec, targets } = control;
  const [target, setTarget] = useState(targets[0] ?? "");
  const [params, setParams] = useState<Record<string, string>>({});
  const [reason, setReason] = useState("");
  const submit = (e: FormEvent) => {
    e.preventDefault();
    onSubmit({ action_type: spec.action_type, target, params: paramValues(spec.params, params), reason });
  };
  return (
    <form onSubmit={submit} aria-label={spec.title} className="space-y-2 border-t border-border px-2.5 py-2 text-xs">
      <p className="text-muted">{spec.description}</p>
      <div className="flex flex-wrap items-end gap-2">
        {targets.length > 1 ? (
          <label className="flex flex-col gap-1.5">
            <span className="label">Target</span>
            <select value={target} onChange={(e) => setTarget(e.target.value)} className="select h-9 text-xs">
              {targets.map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
          </label>
        ) : (
          <p className="flex h-9 items-center font-mono text-[11px] text-muted">{target}</p>
        )}
        <ParamFields params={spec.params} values={params} onChange={(n, v) => setParams((cur) => ({ ...cur, [n]: v }))} compact />
      </div>
      <label className="flex flex-col gap-1.5">
        <span className="label">Reason</span>
        <input
          required
          minLength={5}
          maxLength={500}
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          placeholder="Recorded in the audit log"
          className="input h-9 text-xs"
        />
      </label>
      <p className="text-[11px] text-muted">Verification: {spec.verification}</p>
      <button type="submit" disabled={pending} className="btn btn-primary btn-sm">
        <Icon name={canRun ? "bolt" : "check"} className="h-3.5 w-3.5" />
        {canRun ? "Run" : "Request approval"}
      </button>
    </form>
  );
}
