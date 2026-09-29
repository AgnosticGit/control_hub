"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { Logo } from "@/components/ui/Icon";
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
    <main className="mx-auto max-w-2xl p-8">
      <div className="flex items-center gap-3">
        <Logo className="h-9 w-9" />
        <h1 className="text-2xl font-semibold tracking-tight">Choose an environment</h1>
      </div>
      {projects.length === 0 ? <p className="card mt-6 p-4 text-sm text-muted">You are not a member of any project.</p> : null}
      <ul className="mt-6 space-y-4">
        {projects.map((p) => (
          <li key={`${p.tenant_id}/${p.id}`} className="card p-5">
            <div className="flex items-center gap-2 font-medium">
              <span className="flex h-6 w-6 items-center justify-center rounded-md bg-violet/20 text-xs font-bold text-violet">
                {p.name.slice(0, 1).toUpperCase()}
              </span>
              {p.name} <span className="badge border-border-strong text-muted">{p.role}</span>
            </div>
            <ul className="mt-3 flex flex-wrap gap-2">
              {p.environments.map((e) => (
                <li key={e.id}>
                  <Link href={`/app/${p.id}/${e.id}`} className="btn h-9 px-4 text-sm">
                    <span className="dot bg-ok" aria-hidden />
                    {e.name}
                    {e.is_demo ? <span className="badge ml-1 border-warn/50 bg-warn/10 text-warn">DEMO</span> : null}
                  </Link>
                </li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
    </main>
  );
}
