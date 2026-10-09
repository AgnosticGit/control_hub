"use client";

import Link from "next/link";
import { AppShell } from "@/components/shell/AppShell";
import { Icon } from "@/components/ui/Icon";
import { can } from "@/lib/permissions";
import { useSession } from "@/lib/session";
import { IntegrationGuide } from "./IntegrationGuide";

export function IntegrationPage({ projectId, environmentId }: { projectId: string; environmentId: string }) {
  const session = useSession();
  const project = session.data?.projects.find((p) => p.id === projectId);
  const env = project?.environments.find((e) => e.id === environmentId);

  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  if (!env) {
    return (
      <main className="flex min-h-screen items-center justify-center p-8">
        <div className="card max-w-md p-6 text-sm">
          <p>This environment does not exist or you do not have access to it.</p>
          <Link href="/app" className="btn mt-4">
            <Icon name="arrowLeft" className="h-3.5 w-3.5" />
            Back to environments
          </Link>
        </div>
      </main>
    );
  }

  return (
    <AppShell
      projectId={projectId}
      environmentId={environmentId}
      active="integration"
    >
      <main className="h-full overflow-auto">
        <div className="mx-auto max-w-4xl space-y-5 p-6">
          <h1 className="text-2xl font-semibold tracking-tight">Connect your services</h1>
          <IntegrationGuide projectId={projectId} environmentId={environmentId} canCreateToken={can(project, "tokens.manage")} />
        </div>
      </main>
    </AppShell>
  );
}
