"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect } from "react";
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
      <h1 className="text-xl font-semibold">Choose an environment</h1>
      {projects.length === 0 ? <p className="mt-4 text-sm text-muted">You are not a member of any project.</p> : null}
      <ul className="mt-6 space-y-4">
        {projects.map((p) => (
          <li key={`${p.tenant_id}/${p.id}`} className="rounded-lg border border-border bg-surface p-4">
            <div className="font-medium">
              {p.name} <span className="text-xs text-muted">({p.role})</span>
            </div>
            <ul className="mt-2 flex flex-wrap gap-2">
              {p.environments.map((e) => (
                <li key={e.id}>
                  <Link href={`/app/${p.id}/${e.id}`} className="rounded border border-border px-3 py-1 text-sm hover:border-accent">
                    {e.name}
                    {e.is_demo ? <span className="ml-2 text-xs text-warn">DEMO</span> : null}
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
