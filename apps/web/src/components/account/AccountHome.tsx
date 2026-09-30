"use client";

import Link from "next/link";
import { Icon, type IconName } from "@/components/ui/Icon";
import { STEPS } from "@/lib/onboarding";
import { useSession } from "@/lib/session";
import { AccountShell, Avatar } from "./AccountShell";
import { useUpdateAccount } from "./useAccount";

/** "Finish setting up" card, shown while onboarding is not finished. */
export function SetupCard() {
  const session = useSession();
  const update = useUpdateAccount();
  if (!session.data || session.data.user.onboarding_done) return null;
  return (
    <section aria-label="Setup" className="card space-y-3 border-accent/40 p-5 text-sm">
      <div className="flex items-center gap-2">
        <Icon name="rocket" className="h-4 w-4 text-accent" />
        <h2 className="text-base font-semibold">Finish setting up ControlHub</h2>
      </div>
      <p className="text-muted">A few optional steps to get the most out of ControlHub. Skip any of them; you can always come back.</p>
      <ol className="space-y-1">
        {STEPS.filter((s) => s.id !== "done").map((s, i) => (
          <li key={s.id} className="flex items-center gap-2">
            <span className="flex h-5 w-5 items-center justify-center rounded-full bg-accent/20 text-[11px] text-accent">{i + 1}</span>
            <Link href={`/app/onboarding?step=${s.id}`} className="hover:text-accent">
              {s.title}
            </Link>
          </li>
        ))}
      </ol>
      <div className="flex flex-wrap gap-2">
        <Link href="/app/onboarding" className="btn btn-primary h-9 px-4">
          Continue setup
        </Link>
        <button type="button" className="btn h-9 px-4" disabled={update.isPending} onClick={() => update.mutate({ onboarding_done: true })}>
          Dismiss
        </button>
      </div>
    </section>
  );
}

function Tile({ href, icon, title, text }: { href: string; icon: IconName; title: string; text: string }) {
  return (
    <Link href={href} className="card group flex flex-col gap-2 p-5 transition-colors hover:border-accent/50">
      <span className="card-title-icon">
        <Icon name={icon} className="h-4 w-4" />
      </span>
      <span className="font-medium group-hover:text-accent">{title}</span>
      <span className="text-sm text-muted">{text}</span>
    </Link>
  );
}

export function AccountHome() {
  const session = useSession();
  const user = session.data?.user;
  const projects = session.data?.projects ?? [];
  const name = user?.display_name || user?.email || "";

  return (
    <AccountShell active="overview" title="Your account" description="Manage your info, security and projects.">
      <div className="flex items-center gap-4">
        <Avatar name={name} className="h-16 w-16 text-2xl" />
        <div>
          <p className="text-lg font-medium">Welcome, {name}</p>
          <p className="text-sm text-muted">{user?.email}</p>
        </div>
      </div>

      <SetupCard />

      <div className="grid gap-4 sm:grid-cols-3">
        <Tile href="/app/account/profile" icon="user" title="Personal info" text="Your name and email." />
        <Tile href="/app/account/security" icon="shield" title="Security" text="Password and where you are signed in." />
        <Tile href="/app/account/projects" icon="folder" title="Projects" text={`${projects.length} project${projects.length === 1 ? "" : "s"} you belong to.`} />
      </div>

      <section aria-labelledby="recent-projects" className="card overflow-hidden">
        <div className="flex items-center justify-between px-5 py-3">
          <h2 id="recent-projects" className="text-base font-semibold">
            Your projects
          </h2>
          <Link href="/app/account/projects" className="text-sm text-accent hover:underline">
            Manage projects
          </Link>
        </div>
        <ul>
          {projects.slice(0, 5).map((p) => {
            const env = p.environments[0];
            return (
              <li key={p.id} className="flex items-center gap-3 border-t border-border px-5 py-3 text-sm">
                <span className="flex h-6 w-6 items-center justify-center rounded-md bg-violet/20 text-xs font-bold text-violet">{p.name.slice(0, 1).toUpperCase()}</span>
                <span className="min-w-0 flex-1 truncate font-medium">{p.name}</span>
                <span className="badge border-border-strong text-muted">{p.role_name}</span>
                {env ? (
                  <Link href={`/app/${encodeURIComponent(p.id)}/${encodeURIComponent(env.id)}`} className="btn btn-sm" aria-label={`Open ${p.name}`}>
                    Open
                  </Link>
                ) : null}
              </li>
            );
          })}
          {projects.length === 0 ? <li className="border-t border-border px-5 py-3 text-sm text-muted">You are not a member of any project yet.</li> : null}
        </ul>
      </section>
    </AccountShell>
  );
}
