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
    <span className={`flex shrink-0 items-center justify-center rounded-full border border-border-strong bg-surface-3 font-semibold text-text ${className}`} aria-hidden>
      {name.trim().slice(0, 1).toUpperCase() || "?"}
    </span>
  );
}

export function AccountShell({ active, title, description, children }: { active: AccountNavId; title: string; description?: string; children: ReactNode }) {
  const session = useSession();
  const logout = useLogout();
  const user = session.data?.user;

  return (
    <div className="grid min-h-screen grid-rows-[auto_1fr] md:grid-cols-[248px_minmax(0,1fr)] md:grid-rows-1">
      <aside className="flex flex-col border-b border-border bg-bg-2 md:border-r md:border-b-0" aria-label="Account">
        <div className="flex items-center gap-2 px-5 pt-4 pb-3 md:pt-5 md:pb-4">
          <Link href="/app" className="flex items-center gap-2.5 text-[17px] font-bold tracking-[-0.01em]">
            <Logo />
            karbird
          </Link>
          <span className="ml-auto md:hidden" />
          {session.data?.system ? (
            <Link href="/app/admin" className="btn btn-icon btn-sm md:hidden" aria-label="karbird admin" title="karbird admin">
              <Icon name="shield" className="h-3.5 w-3.5" />
            </Link>
          ) : null}
          <Link href="/app" className="btn btn-icon btn-sm md:hidden" aria-label="Back to projects" title="Back to projects">
            <Icon name="arrowLeft" className="h-3.5 w-3.5" />
          </Link>
          <button type="button" onClick={() => void logout()} className="btn btn-icon btn-sm md:hidden" aria-label="Sign out" title="Sign out">
            <Icon name="logout" className="h-3.5 w-3.5" />
          </button>
        </div>
        <p className="hidden px-5 pt-3 pb-2 text-xs font-medium text-muted md:block">Account</p>
        <nav aria-label="Account sections" className="flex flex-wrap gap-0.5 px-3 pb-3 md:block md:space-y-0.5 md:pb-0">
          {NAV.map((n) => (
            <Link
              key={n.id}
              href={n.href}
              aria-current={active === n.id ? "page" : undefined}
              className={`flex shrink-0 items-center gap-3 rounded-md px-3 py-2 text-sm transition-colors ${
                active === n.id ? "bg-surface-3 font-semibold text-text" : "text-muted hover:bg-surface-2 hover:text-text"
              }`}
            >
              <Icon name={n.icon} className="h-4 w-4" />
              {n.label}
            </Link>
          ))}
        </nav>
        <div className="mt-auto hidden space-y-1 p-3 pt-6 md:block">
          {session.data?.system ? (
            <Link href="/app/admin" className="flex items-center gap-3 rounded-md px-3 py-2 text-sm text-muted hover:bg-surface-2 hover:text-text">
              <Icon name="shield" className="h-4 w-4" />
              karbird admin
            </Link>
          ) : null}
          <Link href="/app" className="flex items-center gap-3 rounded-md px-3 py-2 text-sm text-muted hover:bg-surface-2 hover:text-text">
            <Icon name="arrowLeft" className="h-4 w-4" />
            Back to projects
          </Link>
          <div className="mt-2 flex items-center gap-2 border-t border-border px-1 pt-3">
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
        <div className="mx-auto max-w-4xl space-y-8 px-6 py-10 md:px-10 md:py-12">
          <div>
            <h1 className="text-[2rem] leading-tight font-bold tracking-[-0.02em]">{title}</h1>
            {description ? <p className="mt-2 text-muted">{description}</p> : null}
          </div>
          {session.isPending ? <p className="text-sm text-muted">Loading…</p> : session.error ? <p className="text-sm text-error">Cannot load your account.</p> : children}
        </div>
      </main>
    </div>
  );
}
