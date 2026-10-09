import type { Schemas } from "@/lib/api";

export type ActionPlan = Schemas["ActionPlanDto"];

/** Parse only scoped action frames; other workspace messages are ignored. */
export function parseActionFrame(data: string): ActionPlan | null {
  try {
    const value: unknown = JSON.parse(data);
    if (!value || typeof value !== "object") return null;
    const message = value as Record<string, unknown>;
    if (message.type !== "action" || !message.plan || typeof message.plan !== "object") return null;
    const plan = message.plan as Record<string, unknown>;
    return typeof plan.id === "string" && typeof plan.state === "string" ? plan as ActionPlan : null;
  } catch {
    return null;
  }
}

export function upsertActionPlan(current: ActionPlan[] | undefined, plan: ActionPlan): ActionPlan[] {
  const plans = current ?? [];
  const index = plans.findIndex((item) => item.id === plan.id);
  return index < 0 ? [plan, ...plans] : plans.map((item) => item.id === plan.id ? plan : item);
}

