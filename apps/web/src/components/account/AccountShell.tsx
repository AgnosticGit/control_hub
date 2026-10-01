"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { Icon, Logo, type IconName } from "@/components/ui/Icon";
import { useLogout, useSession } from "@/lib/session";

export type AccountNavId = "overview" | "profile" | "security" | "projects";

const NAV: Array<{ id: AccountNavId; href: string; label: string; icon: IconName }> = [
  { id: "overview", href: "/app/account", label: "Home", icon: "home" },
  { id: "profile", href: "/app/account/profile", label: "Personal info", icon: "user" },
  { id: "security", href: "/app/account/security", label: "Security", icon: "shield" },
  { id: "projects", href: "/app/account/projects", label: "Projects", icon: "folder" },
];

/** Initials-style avatar from a display name or email. */
export function Avatar({ name, className = "h-8 w-8 text-xs" }: { name: string; className?: string }) {
  return (
    <span className={`flex shrink-0 items-center justify-center rounded-full bg-accent/20 font-semibold text-accent ${className}`} aria-hidden>
      {name.trim().slice(0, 1).toUpperCase() || "?"}
    </span>
  );
}

export function AccountShell({ active, title, description, children }: { active: AccountNavId; title: string; description?: string; children: ReactNode }) {
  const session = useSession();
  const logout = useLogout();
  const user = session.data?.user;

  return (
    <div className="grid min-h-screen grid-cols-[240px_minmax(0,1fr)]">
      <aside className="flex flex-col border-r border-border bg-bg-2/80" aria-label="Account">
        <Link href="/app" className="flex items-center gap-2.5 px-5 pt-5 pb-4 text-[17px] font-semibold tracking-tight">
          <Logo />
          ControlHub
        </Link>
        <nav aria-label="Account sections" className="mt-2 space-y-0.5 px-3">
          {NAV.map((n) => (
            <Link
              key={n.id}
              href={n.href}
              aria-current={active === n.id ? "page" : undefined}
              className={`flex items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors ${
                active === n.id ? "bg-accent/15 text-text shadow-[inset_2px_0_0_0_var(--color-accent)]" : "text-muted hover:bg-surface-2 hover:text-text"
              }`}
            >
              <Icon name={n.icon} className={`h-4 w-4 ${active === n.id ? "text-accent" : ""}`} />
              {n.label}
            </Link>
          ))}
        </nav>
        <div className="mt-auto space-y-2 p-3">
          {session.data?.system ? (
            <Link href="/app/admin" className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-muted hover:bg-surface-2 hover:text-text">
              <Icon name="shield" className="h-4 w-4" />
              ControlHub admin
            </Link>
          ) : null}
          <Link href="/app" className="flex items-center gap-2 rounded-lg px-3 py-2 text-sm text-muted hover:bg-surface-2 hover:text-text">
            <Icon name="arrowLeft" className="h-4 w-4" />
            Back to projects
          </Link>
          <div className="flex items-center gap-2 rounded-xl border border-border bg-surface/70 p-2">
            <Avatar name={user?.display_name || user?.email || ""} className="h-7 w-7 text-xs" />
            <span className="min-w-0 flex-1 truncate text-xs text-muted" title={user?.email}>
              {user?.email}
            </span>
            <button type="button" onClick={() => void logout()} className="btn btn-icon btn-sm" aria-label="Sign out" title="Sign out">
              <Icon name="logout" className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </aside>
      <main className="min-w-0 overflow-auto">
        <div className="mx-auto max-w-4xl space-y-6 p-8">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
            {description ? <p className="mt-1 text-sm text-muted">{description}</p> : null}
          </div>
          {session.isPending ? <p className="text-sm text-muted">Loading…</p> : session.error ? <p className="text-sm text-error">Cannot load your account.</p> : children}
        </div>
      </main>
    </div>
  );
}
