"use client";

import { useQuery } from "@tanstack/react-query";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useUpdateAccount } from "@/components/account/useAccount";
import { IntegrationGuide, useSeenServices } from "@/components/integration/IntegrationGuide";
import { InviteForm } from "@/components/settings/InvitesTab";
import type { Actor } from "@/components/settings/matrix";
import { ErrorNotice } from "@/components/ui/ErrorNotice";
import { Icon, Logo } from "@/components/ui/Icon";
import { api, unwrap, type ProjectDto } from "@/lib/api";
import { nextStep, onboardingTarget, parseStep, STEPS, type StepId } from "@/lib/onboarding";
import { can } from "@/lib/permissions";
import { useSession } from "@/lib/session";

function Stepper({ step }: { step: StepId }) {
  const current = STEPS.findIndex((s) => s.id === step);
  return (
    <ol aria-label="Setup steps" className="flex items-center gap-2 text-xs">
      {STEPS.map((s, i) => (
        <li key={s.id} aria-current={i === current ? "step" : undefined} className="flex items-center gap-2">
          <span
            className={`flex h-6 w-6 items-center justify-center rounded-full border text-[11px] ${
              i < current ? "border-ok/60 bg-ok/15 text-ok" : i === current ? "border-accent bg-accent/20 text-accent" : "border-border text-muted"
            }`}
          >
            {i < current ? <Icon name="check" className="h-3 w-3" /> : i + 1}
          </span>
          <span className={i === current ? "text-text" : "text-muted"}>{s.title}</span>
          {i < STEPS.length - 1 ? <span className="mx-1 h-px w-6 bg-border" aria-hidden /> : null}
        </li>
      ))}
    </ol>
  );
}

function InviteStep({ project, userId }: { project: ProjectDto; userId: string }) {
  const manage = can(project, "members.manage");
  const roles = useQuery({
    queryKey: ["roles", project.id],
    enabled: manage,
    queryFn: async () => unwrap(await api.GET("/api/v1/projects/{project_id}/roles", { params: { path: { project_id: project.id } } })),
  });
  if (!manage) {
    return <p className="card p-5 text-sm text-muted">Inviting people needs the “Manage members and their roles” permission in {project.name}.</p>;
  }
  const actor: Actor = { userId, permissions: project.permissions, isOwner: project.role === "owner" };
  return (
    <section className="card space-y-3 p-5 text-sm">
      <p className="text-muted">
        Create a link and send it to your teammates. Whoever opens it signs in or creates an account and joins {project.name} with the role you
        choose. You can change their permissions later under Members &amp; roles.
      </p>
      {roles.data ? <InviteForm projectId={project.id} actor={actor} roles={roles.data} /> : <p className="text-muted">Loading roles…</p>}
    </section>
  );
}

export function Onboarding() {
  const router = useRouter();
  const params = useSearchParams();
  const step = parseStep(params.get("step"));
  const session = useSession();
  const update = useUpdateAccount();
  const target = onboardingTarget(session.data?.projects ?? [], params.get("project"));
  const project = target?.project;
  const env = target?.environment;
  const seen = useSeenServices(can(project, "telemetry.view") ? (project?.id ?? "") : "", env?.id ?? "");
  const receiving = (seen.data?.services.length ?? 0) > 0;
  const overview = project && env ? `/app/${encodeURIComponent(project.id)}/${encodeURIComponent(env.id)}` : "/app";

  const go = (s: StepId) => {
    const q = new URLSearchParams({ step: s });
    if (project) q.set("project", project.id);
    router.push(`/app/onboarding?${q.toString()}`);
  };
  const finish = (to: string) => update.mutate({ onboarding_done: true }, { onSuccess: () => router.push(to) });

  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  const name = session.data?.user.display_name || session.data?.user.email || "";
  const title = STEPS.find((s) => s.id === step)?.title ?? "";

  return (
    <div className="min-h-screen">
      <header className="flex flex-wrap items-center gap-4 border-b border-border bg-bg-2/60 px-6 py-3">
        <Link href="/app" className="flex items-center gap-2 font-semibold">
          <Logo className="h-6 w-6" />
          karbird
        </Link>
        <div className="mx-auto">
          <Stepper step={step} />
        </div>
        <button type="button" className="btn btn-sm" disabled={update.isPending} onClick={() => finish(overview)}>
          Skip setup
        </button>
      </header>

      <main className="mx-auto max-w-4xl space-y-5 p-6">
        <div>
          {step === "connect" ? <p className="text-sm text-muted">Welcome to karbird, {name}. Setup takes a few minutes; every step is optional.</p> : null}
          <h1 className="mt-1 text-2xl font-semibold tracking-tight">{step === "done" ? "You're all set" : title}</h1>
          {project ? <p className="mt-1 text-sm text-muted">Project: {project.name}{env ? ` / ${env.name}` : ""}</p> : null}
        </div>

        {update.error ? <ErrorNotice className="text-sm">Could not save your progress. Try again.</ErrorNotice> : null}

        {!project || !env ? (
          <p className="card p-5 text-sm">
            You are not a member of any project yet.{" "}
            <Link href="/app/account/projects" className="text-accent hover:underline">
              Create a project
            </Link>{" "}
            to connect your system.
          </p>
        ) : step === "connect" ? (
          <IntegrationGuide projectId={project.id} environmentId={env.id} canCreateToken={can(project, "tokens.manage")} />
        ) : step === "invite" ? (
          <InviteStep project={project} userId={session.data?.user.id ?? ""} />
        ) : (
          <section className="card space-y-3 p-5 text-sm">
            <ul className="space-y-2">
              <li className="flex items-center gap-2">
                <span className={`dot ${receiving ? "bg-ok" : "bg-muted"}`} aria-hidden />
                {receiving
                  ? `Receiving telemetry from ${seen.data?.services.map((s) => s.name).join(", ")}.`
                  : "No telemetry yet. The Integration page has the same guide whenever you are ready."}
              </li>
              <li className="flex items-center gap-2">
                <span className="dot bg-ok" aria-hidden />
                Invite more people any time from Members &amp; roles; manage your profile and projects in your account.
              </li>
            </ul>
          </section>
        )}

        <div className="flex flex-wrap items-center justify-end gap-2">
          {step === "done" ? (
            <>
              <button type="button" className="btn h-10 px-4" disabled={update.isPending} onClick={() => finish("/app/account")}>
                Go to your account
              </button>
              <button type="button" className="btn btn-primary h-10 px-5" disabled={update.isPending} onClick={() => finish(overview)}>
                Open {project?.name ?? "karbird"}
                <Icon name="arrowRight" className="h-4 w-4" />
              </button>
            </>
          ) : (
            <>
              {step === "connect" && receiving ? null : (
                <button type="button" className="btn h-10 px-4" onClick={() => go(nextStep(step))}>
                  Skip this step
                </button>
              )}
              {step === "connect" && !receiving ? null : (
                <button type="button" className="btn btn-primary h-10 px-5" onClick={() => go(nextStep(step))}>
                  Continue
                  <Icon name="arrowRight" className="h-4 w-4" />
                </button>
              )}
            </>
          )}
        </div>
      </main>
    </div>
  );
}
