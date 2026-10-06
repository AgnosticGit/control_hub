import type { Schemas } from "@/lib/api";

type Spec = Schemas["ActionSpecDto"];
type Plan = Schemas["ActionPlanDto"];

export const OPEN_STATES: ReadonlySet<string> = new Set(["proposed", "approved", "running"]);

export function stateClass(state: string): string {
  if (state === "succeeded") return "text-ok";
  if (state === "failed" || state === "verification_failed") return "text-error";
  if (OPEN_STATES.has(state)) return "text-warn";
  return "text-muted";
}

/** A map card (by its name) or a container (by its name in the infrastructure layer). */
export type ControlSubject = { service: string } | { container: string };

export interface Control {
  spec: Spec;
  /** Targets of this action that belong to the subject, in catalog order. */
  targets: string[];
}

/**
 * Catalog actions that may run here and act on the subject. A service card offers everything
 * aimed at the service, its feature flags and its containers; a container offers only what is
 * aimed at that container.
 */
export function controlsFor(specs: readonly Spec[], subject: ControlSubject): Control[] {
  const out: Control[] = [];
  for (const spec of specs) {
    if (!spec.allowed_here) continue;
    const targets = spec.subjects
      .filter((s) => ("service" in subject ? s.service === subject.service : s.container === subject.container))
      .map((s) => s.target);
    if (targets.length > 0) out.push({ spec, targets });
  }
  return out;
}

/** Newest plans of the controls' actions on their targets (the API lists newest first). */
export function plansFor(plans: readonly Plan[], controls: readonly Control[], limit = 5): Plan[] {
  const keys = new Set(controls.flatMap((c) => c.targets.map((t) => `${c.spec.action_type} ${t}`)));
  return plans.filter((p) => keys.has(`${p.action_type} ${p.target}`)).slice(0, limit);
}
