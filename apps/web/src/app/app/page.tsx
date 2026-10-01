"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { SetupCard } from "@/components/account/AccountHome";
import { projectColor } from "@/components/account/project-color";
import { Icon, Logo } from "@/components/ui/Icon";
import { useSession } from "@/lib/session";

export default function AppIndex() {
  const router = useRouter();
  const session = useSession();
  const projects = session.data?.projects ?? [];
  const only = projects.length === 1 && projects[0]?.environments.length === 1 ? projects[0] : null;

  useEffect(() => {
    const env = only?.environments[0];
    if (only && env) router.replace(`/app/${only.id}/${env.id}`);
  }, [only, router]);

  if (session.isPending || only) {
    return <p className="p-8 text-sm text-muted">Loading…</p>;
  }
  if (session.error) {
    return <p className="p-8 text-sm text-error">Cannot load session.</p>;
  }
  return (
    <div className="min-h-screen">
      <header className="border-b border-border">
        <div className="mx-auto flex max-w-4xl items-center gap-3 px-6 py-3">
          <Link href="/app" className="flex shrink-0 items-center gap-2.5 text-[17px] font-bold tracking-[-0.01em]">
            <Logo />
            ControlHub
          </Link>
          <nav aria-label="Account" className="ml-auto flex gap-2">
            <Link href="/app/account/projects" className="btn btn-sm">
              <Icon name="plus" className="h-3.5 w-3.5" />
              <span className="max-sm:sr-only">New project</span>
            </Link>
            <Link href="/app/account" className="btn btn-sm">
              <Icon name="user" className="h-3.5 w-3.5" />
              <span className="max-sm:sr-only">Your account</span>
            </Link>
            {session.data?.system ? (
              <Link href="/app/admin" className="btn btn-sm">
                <Icon name="shield" className="h-3.5 w-3.5" />
                <span className="max-sm:sr-only">ControlHub admin</span>
              </Link>
            ) : null}
          </nav>
        </div>
      </header>

      <main className="mx-auto max-w-4xl px-6 py-12">
        <h1 className="text-[2rem] leading-tight font-bold tracking-[-0.02em]">Choose an environment</h1>
        <p className="mt-2 text-muted">
          {projects.length === 0
            ? "You are not a member of any project yet."
            : `${projects.length} project${projects.length === 1 ? "" : "s"} you belong to. Open an environment to see its live map.`}
        </p>

        <div className="mt-8 empty:hidden">
          <SetupCard />
        </div>

        {projects.length === 0 ? (
          <div className="mt-8 rounded-lg border border-dashed border-border-strong p-6 text-sm text-muted">
            You are not a member of any project.{" "}
            <Link href="/app/account/projects" className="font-semibold text-text underline decoration-border-strong hover:decoration-text">
              Create one
            </Link>{" "}
            or ask a project owner for an invite link.
          </div>
        ) : null}

        <ul className="mt-8 divide-y divide-border border-y border-border">
          {projects.map((p) => {
            const color = projectColor(p.id);
            return (
              <li key={`${p.tenant_id}/${p.id}`} className="grid gap-4 py-6 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)] sm:gap-8">
                <div className="flex items-start gap-3">
                  <span className="mt-2 h-1.5 w-6 shrink-0 rounded-full" style={{ background: color }} aria-hidden />
                  <div className="min-w-0">
                    <p className="truncate text-base font-semibold">{p.name}</p>
                    <p className="mt-0.5 text-sm text-muted">{p.role_name}</p>
                  </div>
                </div>
                <ul className="flex flex-wrap gap-2" aria-label={`Environments of ${p.name}`}>
                  {p.environments.map((e) => (
                    <li key={e.id}>
                      <Link
                        href={`/app/${p.id}/${e.id}`}
                        className="group flex h-10 items-center gap-2.5 rounded-md border border-border bg-surface px-3.5 text-sm font-medium transition-colors hover:border-border-strong hover:bg-surface-2"
                      >
                        <span className="h-3 w-3 rounded-full border-[2.5px] bg-bg" style={{ borderColor: color }} aria-hidden />
                        {e.name}
                        {e.is_demo ? <span className="badge ml-1 border-border-strong text-muted">DEMO</span> : null}
                        <Icon name="arrowRight" className="h-3.5 w-3.5 text-muted transition-transform group-hover:translate-x-0.5" />
                      </Link>
                    </li>
                  ))}
                  {p.environments.length === 0 ? <li className="text-sm text-muted">No environments yet.</li> : null}
                </ul>
              </li>
            );
          })}
        </ul>
      </main>
    </div>
  );
}
