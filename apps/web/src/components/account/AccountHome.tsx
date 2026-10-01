"use client";

import Link from "next/link";
import { Icon } from "@/components/ui/Icon";
import { STEPS } from "@/lib/onboarding";
import { useSession } from "@/lib/session";
import { AccountShell, Avatar } from "./AccountShell";
import { projectColor } from "./project-color";
import { useUpdateAccount } from "./useAccount";

/** "Finish setting up" panel, shown while onboarding is not finished. */
export function SetupCard() {
  const session = useSession();
  const update = useUpdateAccount();
  if (!session.data || session.data.user.onboarding_done) return null;
  const steps = STEPS.filter((s) => s.id !== "done");
  return (
    <section aria-label="Setup" className="rounded-lg border border-border bg-surface p-6 text-sm">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="max-w-md">
          <h2 className="text-base font-semibold">Finish setting up ControlHub</h2>
          <p className="mt-1 leading-relaxed text-muted">A few optional steps to get the most out of ControlHub. Skip any of them; you can always come back.</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Link href="/app/onboarding" className="btn btn-primary h-9 px-4">
            Continue setup
          </Link>
          <button type="button" className="btn h-9 px-4" disabled={update.isPending} onClick={() => update.mutate({ onboarding_done: true })}>
            Dismiss
          </button>
        </div>
      </div>
      <ol className="relative mt-6 grid gap-y-3 sm:auto-cols-fr sm:grid-flow-col sm:gap-y-0">
        <span aria-hidden className="absolute top-0 bottom-0 left-[6px] w-[3px] rounded-full bg-border-strong sm:top-[6px] sm:right-0 sm:bottom-auto sm:h-[3px] sm:w-auto" />
        {steps.map((s, i) => (
          <li key={s.id} className="relative flex items-center gap-3 sm:block">
            <span aria-hidden className="relative z-[1] block h-[15px] w-[15px] rounded-full border-[3px] border-text bg-surface" />
            <Link href={`/app/onboarding?step=${s.id}`} className="font-medium hover:underline sm:mt-2.5 sm:block">
              <span className="text-muted tabular-nums">{i + 1}.</span> {s.title}
            </Link>
          </li>
        ))}
      </ol>
    </section>
  );
}

export function AccountHome() {
  const session = useSession();
  const user = session.data?.user;
  const projects = session.data?.projects ?? [];
  const name = user?.display_name || user?.email || "";

  const rows = [
    { href: "/app/account/profile", icon: "user" as const, title: "Personal info", value: user?.display_name ? `${user.display_name} · ${user.email}` : (user?.email ?? "") },
    { href: "/app/account/security", icon: "shield" as const, title: "Security", value: "Password and where you are signed in" },
    {
      href: "/app/account/projects",
      icon: "folder" as const,
      title: "Projects",
      value: `${projects.length} project${projects.length === 1 ? "" : "s"} you belong to`,
    },
  ];

  return (
    <AccountShell active="overview" title="Your account" description="Manage your info, security and projects.">
      <div className="flex items-center gap-5">
        <Avatar name={name} className="h-14 w-14 text-xl" />
        <div className="min-w-0">
          <p className="truncate text-xl font-semibold tracking-[-0.01em]">Welcome, {name}</p>
          <p className="truncate text-sm text-muted">{user?.email}</p>
        </div>
      </div>

      <SetupCard />

      <nav aria-label="Account overview" className="divide-y divide-border rounded-lg border border-border bg-surface">
        {rows.map((r) => (
          <Link key={r.href} href={r.href} className="group flex items-center gap-4 px-5 py-4 transition-colors first:rounded-t-lg last:rounded-b-lg hover:bg-surface-2">
            <Icon name={r.icon} className="h-4 w-4 shrink-0 text-muted" />
            <span className="w-32 shrink-0 text-sm font-semibold">{r.title}</span>
            <span className="min-w-0 flex-1 truncate text-sm text-muted max-sm:invisible">{r.value}</span>
            <Icon name="arrowRight" className="h-4 w-4 shrink-0 text-muted transition-transform group-hover:translate-x-0.5 group-hover:text-text" />
          </Link>
        ))}
      </nav>

      <section aria-labelledby="recent-projects">
        <div className="flex items-baseline justify-between gap-4">
          <h2 id="recent-projects" className="text-lg font-semibold">
            Your projects
          </h2>
          <Link href="/app/account/projects" className="text-sm font-medium text-muted underline decoration-border-strong hover:text-text hover:decoration-text">
            Manage projects
          </Link>
        </div>
        <ul className="mt-3 divide-y divide-border border-y border-border">
          {projects.slice(0, 5).map((p) => {
            const env = p.environments[0];
            return (
              <li key={p.id} className="flex items-center gap-4 py-3.5 text-sm">
                <span className="h-1.5 w-6 shrink-0 rounded-full" style={{ background: projectColor(p.id) }} aria-hidden />
                <span className="min-w-0 flex-1 truncate font-semibold">{p.name}</span>
                <span className="text-muted">{p.role_name}</span>
                {env ? (
                  <Link href={`/app/${encodeURIComponent(p.id)}/${encodeURIComponent(env.id)}`} className="btn btn-sm" aria-label={`Open ${p.name}`}>
                    Open
                  </Link>
                ) : null}
              </li>
            );
          })}
          {projects.length === 0 ? <li className="py-3.5 text-sm text-muted">You are not a member of any project yet.</li> : null}
        </ul>
      </section>
    </AccountShell>
  );
}
