"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import type { ReactNode } from "react";
import { Icon, Logo, type IconName } from "@/components/ui/Icon";
import { can } from "@/lib/permissions";
import { useLogout, useSession } from "@/lib/session";
import { workspaceTarget } from "./scope-switcher";

export type NavId = "overview" | "integration" | "actions" | "tokens" | "settings";

interface Props {
  projectId: string;
  environmentId: string;
  active: NavId;
  /** Top bar content (search, range, …). */
  top?: ReactNode;
  /** Sidebar "System status" block content. */
  status?: ReactNode;
  children: ReactNode;
}

export function AppShell({ projectId, environmentId, active, top, status, children }: Props) {
  const session = useSession();
  const router = useRouter();
  const pathname = usePathname();
  const logout = useLogout();
  const project = session.data?.projects.find((p) => p.id === projectId);
  const base = `/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}`;
  const email = session.data?.user.email ?? "";
  const organization = session.data?.organizations.find((item) => item.id === project?.organization_id);
  const organizationProjects = session.data?.projects.filter((item) => item.organization_id === project?.organization_id) ?? [];
  const navigate = (nextProjectId: string, nextEnvironmentId?: string) => {
    const nextProject = session.data?.projects.find((item) => item.id === nextProjectId);
    const targetEnvironment = nextEnvironmentId ?? nextProject?.environments[0]?.id;
    router.push(targetEnvironment ? workspaceTarget(nextProjectId, targetEnvironment, pathname) : "/app");
  };

  const nav: Array<{ id: NavId; href: string; label: string; icon: IconName; show: boolean }> = [
    { id: "overview", href: base, label: "Overview", icon: "map", show: true },
    { id: "integration", href: `${base}/integration`, label: "Integration", icon: "file", show: true },
    { id: "actions", href: `${base}/actions`, label: "Actions", icon: "bolt", show: can(project, "actions.view") },
    { id: "tokens", href: `${base}/tokens`, label: "Tokens", icon: "key", show: can(project, "tokens.view") },
    { id: "settings", href: `${base}/settings`, label: "Members & roles", icon: "users", show: can(project, "members.view") },
  ];

  return (
    <div className="grid h-screen grid-cols-[228px_minmax(0,1fr)] overflow-hidden">
      <aside className="flex min-h-0 flex-col border-r border-border bg-bg-2/80" aria-label="Sidebar">
        <Link href="/app" className="flex items-center gap-2.5 px-5 pt-5 pb-4 text-[17px] font-semibold tracking-tight">
          <Logo />
          karbird
        </Link>

        <div className="mx-3 grid gap-2 rounded-xl border border-border bg-surface/70 p-3">
          <label className="grid gap-1 text-xs text-muted">
            Organization
            <select
              aria-label="Organization"
              className="input h-8 text-xs"
              value={organization?.id ?? ""}
              onChange={(event) => {
                const next = session.data?.projects.find((item) => item.organization_id === event.target.value);
                if (next) navigate(next.id);
                else router.push("/app");
              }}
            >
              {(session.data?.organizations ?? []).map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
            </select>
          </label>
          <label className="grid gap-1 text-xs text-muted">
            Project
            <select aria-label="Project" className="input h-8 text-xs" value={projectId} onChange={(event) => navigate(event.target.value)}>
              {organizationProjects.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
            </select>
          </label>
          <label className="grid gap-1 text-xs text-muted">
            Environment
            <select aria-label="Environment" className="input h-8 text-xs" value={environmentId} onChange={(event) => navigate(projectId, event.target.value)}>
              {(project?.environments ?? []).map((item) => <option key={item.id} value={item.id}>{item.name}{item.is_demo ? " · demo" : ""}</option>)}
            </select>
          </label>
        </div>

        <nav aria-label="Workspace" className="mt-4 space-y-0.5 px-3">
          {nav
            .filter((n) => n.show)
            .map((n) => (
              <Link
                key={n.id}
                href={n.href}
                aria-current={active === n.id ? "page" : undefined}
                className={`flex items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors ${
                  active === n.id
                    ? "bg-accent/15 text-text shadow-[inset_2px_0_0_0_var(--color-accent)]"
                    : "text-muted hover:bg-surface-2 hover:text-text"
                }`}
              >
                <Icon name={n.icon} className={`h-4 w-4 ${active === n.id ? "text-accent" : ""}`} />
                {n.label}
              </Link>
            ))}
        </nav>

        <div className="mt-auto space-y-3 p-3">
          {session.data && !session.data.user.onboarding_done ? (
            <Link
              href={`/app/onboarding?project=${encodeURIComponent(projectId)}`}
              className="flex items-center gap-2 rounded-xl border border-accent/40 bg-accent/10 px-3 py-2 text-sm text-text hover:bg-accent/15"
            >
              <Icon name="rocket" className="h-4 w-4 text-accent" />
              Finish setup
            </Link>
          ) : null}
          {session.data?.system ? (
            <Link
              href="/app/admin"
              className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-muted hover:bg-surface-2 hover:text-text"
            >
              <Icon name="shield" className="h-4 w-4" />
              karbird admin
            </Link>
          ) : null}
          {status ? (
            <div className="rounded-xl border border-border bg-surface/70 p-3">
              <div className="label mb-2">System status</div>
              <div className="space-y-1.5 text-xs">{status}</div>
            </div>
          ) : null}
          <div className="flex items-center gap-2 rounded-xl border border-border bg-surface/70 p-2">
            <Link href="/app/account" className="flex min-w-0 flex-1 items-center gap-2 rounded-lg hover:text-text" aria-label="Your account" title={email}>
              <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-accent/20 text-xs font-semibold text-accent">
                {email.slice(0, 1).toUpperCase() || "?"}
              </span>
              <span className="min-w-0 flex-1 truncate text-xs text-muted">{email}</span>
            </Link>
            <button type="button" onClick={() => void logout()} className="btn btn-icon btn-sm" aria-label="Sign out" title="Sign out">
              <Icon name="logout" className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </aside>

      <div className="flex min-h-0 min-w-0 flex-col">
        {top ? <header className="flex h-14 shrink-0 items-center gap-3 border-b border-border bg-bg-2/60 px-4 backdrop-blur">{top}</header> : null}
        <div className="min-h-0 flex-1">{children}</div>
      </div>
    </div>
  );
}

/** Row in the sidebar "System status" block. */
export function StatusRow({ tone, children }: { tone: "ok" | "warn" | "error" | "muted"; children: ReactNode }) {
  const color = { ok: "bg-ok", warn: "bg-warn", error: "bg-error", muted: "bg-muted" }[tone];
  return (
    <div className="flex items-center gap-2">
      <span className={`dot ${color}`} aria-hidden />
      <span className="min-w-0 flex-1">{children}</span>
    </div>
  );
}
