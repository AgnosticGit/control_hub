// Onboarding is offered after sign-up and never required: every step can be skipped.

export const STEPS = [
  { id: "connect", title: "Connect your system" },
  { id: "invite", title: "Invite your team" },
  { id: "done", title: "All set" },
] as const;

export type StepId = (typeof STEPS)[number]["id"];

export function parseStep(raw: string | null | undefined): StepId {
  return STEPS.find((s) => s.id === raw)?.id ?? "connect";
}

export function nextStep(step: StepId): StepId {
  const i = STEPS.findIndex((s) => s.id === step);
  return STEPS[Math.min(i + 1, STEPS.length - 1)]?.id ?? "done";
}

type Env = { id: string; is_demo: boolean };
type Project<E extends Env> = { id: string; role: string; permissions: readonly string[]; environments: readonly E[] };

/** The project and environment to set up: one you own (a fresh sign-up owns exactly one), a real
 *  environment before a demo one; `preferred` wins when you are a member of it. */
export function onboardingTarget<P extends Project<Env>>(
  projects: readonly P[],
  preferred?: string | null,
): { project: P; environment: P["environments"][number] } | null {
  const withEnv = projects.filter((p) => p.environments.length > 0);
  const project =
    withEnv.find((p) => p.id === preferred) ?? withEnv.find((p) => p.role === "owner" && p.environments.some((e) => !e.is_demo)) ?? withEnv.find((p) => p.role === "owner") ?? withEnv[0];
  if (!project) return null;
  const environment = project.environments.find((e) => !e.is_demo) ?? project.environments[0];
  return environment ? { project, environment } : null;
}
