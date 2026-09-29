"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { Icon, Logo, type IconName } from "@/components/ui/Icon";
import { useLogout, useSession } from "@/lib/session";

export type NavId = "overview" | "actions" | "tokens";

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
  const logout = useLogout();
  const project = session.data?.projects.find((p) => p.id === projectId);
  const env = project?.environments.find((e) => e.id === environmentId);
  const isAdmin = project?.role === "admin" || project?.role === "owner";
  const base = `/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}`;
  const email = session.data?.user.email ?? "";

  const nav: Array<{ id: NavId; href: string; label: string; icon: IconName; show: boolean }> = [
    { id: "overview", href: base, label: "Overview", icon: "map", show: true },
    { id: "actions", href: `${base}/actions`, label: "Actions", icon: "bolt", show: true },
    { id: "tokens", href: `${base}/tokens`, label: "Tokens", icon: "key", show: isAdmin },
  ];

  return (
    <div className="grid h-screen grid-cols-[228px_minmax(0,1fr)] overflow-hidden">
      <aside className="flex min-h-0 flex-col border-r border-border bg-bg-2/80" aria-label="Sidebar">
        <Link href="/app" className="flex items-center gap-2.5 px-5 pt-5 pb-4 text-[17px] font-semibold tracking-tight">
          <Logo />
          ControlHub
        </Link>

        <div className="mx-3 space-y-2 rounded-xl border border-border bg-surface/70 p-3">
          <div>
            <div className="label">Project</div>
            <div className="mt-1 flex items-center gap-2 text-sm font-medium">
              <span className="flex h-5 w-5 items-center justify-center rounded-md bg-violet/20 text-[10px] font-bold text-violet">
                {(project?.name ?? projectId).slice(0, 1).toUpperCase()}
              </span>
              <span className="truncate">{project?.name ?? projectId}</span>
            </div>
          </div>
          <div>
            <div className="label">Environment</div>
            <div className="mt-1 flex items-center gap-2 text-sm">
              <span className="dot bg-ok shadow-[0_0_8px_rgb(34_197_94/0.8)]" aria-hidden />
              <span className="truncate">{env?.name ?? environmentId}</span>
              {env?.is_demo ? (
                <span className="badge ml-auto border-warn/50 bg-warn/10 text-warn" title="Demo environment with synthetic load and fault injection">
                  DEMO
                </span>
              ) : null}
            </div>
          </div>
          <Link href="/app" className="block text-[11px] text-muted hover:text-accent">
            Switch environment
          </Link>
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
          {status ? (
            <div className="rounded-xl border border-border bg-surface/70 p-3">
              <div className="label mb-2">System status</div>
              <div className="space-y-1.5 text-xs">{status}</div>
            </div>
          ) : null}
          <div className="flex items-center gap-2 rounded-xl border border-border bg-surface/70 p-2">
            <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-accent/20 text-xs font-semibold text-accent">
              {email.slice(0, 1).toUpperCase() || "?"}
            </span>
            <span className="min-w-0 flex-1 truncate text-xs text-muted" title={email}>
              {email}
            </span>
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
