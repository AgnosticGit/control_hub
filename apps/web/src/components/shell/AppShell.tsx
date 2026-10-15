"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon, Logo, type IconName } from "@/components/ui/Icon";
import { can } from "@/lib/permissions";
import { usePersisted, writePersisted } from "@/lib/persisted";
import { useLogout, useSession } from "@/lib/session";
import { workspaceTarget } from "./scope-switcher";

export type NavId = "overview" | "incidents" | "changes" | "integration" | "actions" | "tokens" | "settings";

interface Props {
  projectId: string;
  environmentId: string;
  active: NavId;
  /** Sidebar "System status" block content. */
  status?: ReactNode;
  children: ReactNode;
}

const SIDEBAR_KEY = "karbird.shell.sidebarHidden";

export function AppShell({ projectId, environmentId, active, status, children }: Props) {
  const session = useSession();
  const router = useRouter();
  const pathname = usePathname();
  const logout = useLogout();
  const sidebarHidden = usePersisted(SIDEBAR_KEY, false, (raw) => raw === "true" ? true : raw === "false" ? false : null);
  const [scopeMenu, setScopeMenu] = useState<string | null>(null);
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
    { id: "incidents", href: `${base}/incidents`, label: "Incidents", icon: "alert", show: can(project, "telemetry.view") },
    { id: "changes", href: `${base}/changes`, label: "Change Replay", icon: "repeat", show: true },
    { id: "integration", href: `${base}/integration`, label: "Integration", icon: "file", show: true },
    { id: "actions", href: `${base}/actions`, label: "Actions", icon: "bolt", show: can(project, "actions.view") },
    { id: "tokens", href: `${base}/tokens`, label: "Tokens", icon: "key", show: can(project, "tokens.view") },
    { id: "settings", href: `${base}/settings`, label: "Members & roles", icon: "users", show: can(project, "members.view") },
  ];

  return (
    <div className="relative grid h-screen overflow-hidden transition-[grid-template-columns] duration-200 ease-out" style={{ gridTemplateColumns: sidebarHidden ? "0px minmax(0,1fr)" : "228px minmax(0,1fr)" }}>
      <aside className={`flex min-h-0 min-w-0 flex-col overflow-hidden bg-bg-2/80 transition-[opacity,transform] duration-200 ${sidebarHidden ? "pointer-events-none border-r-0 opacity-0" : "border-r border-border opacity-100"}`} aria-label="Sidebar" aria-hidden={sidebarHidden} inert={sidebarHidden}>
        <div className="flex shrink-0 items-center justify-between px-4 pt-4 pb-3">
          <Link href="/app" className="flex items-center gap-2.5 text-[17px] font-semibold tracking-tight">
            <Logo />
            karbird
          </Link>
        </div>

        <div className="mx-3 grid gap-2 rounded-xl border border-border bg-surface/70 p-3">
          <ScopeSelect label="Organization" value={organization?.name ?? "Select organization"} id="organization" open={scopeMenu === "organization"} onOpenChange={(open) => setScopeMenu(open ? "organization" : null)} options={(session.data?.organizations ?? []).map((item) => ({ id: item.id, label: item.name }))} selectedId={organization?.id ?? ""} onSelect={(id) => {
                const next = session.data?.projects.find((item) => item.organization_id === id);
                if (next) navigate(next.id);
                else router.push("/app");
          }} />
          <ScopeSelect label="Project" value={project?.name ?? "Select project"} id="project" open={scopeMenu === "project"} onOpenChange={(open) => setScopeMenu(open ? "project" : null)} options={organizationProjects.map((item) => ({ id: item.id, label: item.name }))} selectedId={projectId} onSelect={(id) => navigate(id)} />
          <ScopeSelect label="Environment" value={
            <span className="flex min-w-0 items-center gap-2">
              <span className="truncate">{project?.environments.find((item) => item.id === environmentId)?.name ?? "Select environment"}</span>
              {project?.environments.find((item) => item.id === environmentId)?.is_demo ? <span className="badge shrink-0">DEMO</span> : null}
            </span>
          } id="environment" open={scopeMenu === "environment"} onOpenChange={(open) => setScopeMenu(open ? "environment" : null)} options={(project?.environments ?? []).map((item) => ({ id: item.id, label: `${item.name}${item.is_demo ? " · demo" : ""}` }))} selectedId={environmentId} onSelect={(id) => navigate(projectId, id)} />
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

      <button
        type="button"
        className={`absolute top-1/2 z-50 flex h-12 w-5 -translate-y-1/2 items-center justify-center rounded-r-lg border border-l-0 border-border bg-surface-2 text-muted shadow-lg transition-[left,color,background] hover:bg-surface-3 hover:text-text ${sidebarHidden ? "left-0" : "left-[228px]"}`}
        aria-label={sidebarHidden ? "Show sidebar" : "Hide sidebar"}
        title={sidebarHidden ? "Show sidebar" : "Hide sidebar"}
        onClick={() => writePersisted(SIDEBAR_KEY, sidebarHidden ? "false" : "true")}
      >
        <Icon name={sidebarHidden ? "panelLeft" : "chevronLeft"} className="h-3.5 w-3.5" />
      </button>
      <div className="flex min-h-0 min-w-0 flex-col">
        <div className="min-h-0 min-w-0 flex-1">{children}</div>
      </div>
    </div>
  );
}

interface ScopeOption { id: string; label: string }

function ScopeSelect({ label, value, id, open, onOpenChange, options, selectedId, onSelect }: {
  label: string; value: ReactNode; id: string; open: boolean; onOpenChange: (open: boolean) => void;
  options: ScopeOption[]; selectedId: string; onSelect: (id: string) => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(Math.max(0, options.findIndex((item) => item.id === selectedId)));
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) onOpenChange(false); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") onOpenChange(false); };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("pointerdown", outside); document.removeEventListener("keydown", escape); };
  }, [open, onOpenChange]);
  const move = (delta: number) => { if (options.length > 0) setActive((index) => (index + delta + options.length) % options.length); };
  return (
    <div className="relative grid gap-1 text-xs text-muted" ref={root}>
      <span>{label}</span>
      <button type="button" role="combobox" className={`flex h-9 w-full min-w-0 items-center justify-between gap-2 rounded-lg border bg-bg/60 px-2.5 text-left text-sm text-text transition-colors hover:border-accent/50 hover:bg-surface-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${open ? "border-accent/60" : "border-border"}`} aria-label={label} aria-haspopup="listbox" aria-expanded={open} aria-controls={`${id}-options`} aria-activedescendant={open && options[active] ? `${id}-option-${active}` : undefined} onClick={() => { setActive(Math.max(0, options.findIndex((item) => item.id === selectedId))); onOpenChange(!open); }} onKeyDown={(event) => {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); if (!open) onOpenChange(true); else move(event.key === "ArrowDown" ? 1 : -1); }
        if (event.key === "Enter" && open) { event.preventDefault(); if (options[active]) onSelect(options[active].id); onOpenChange(false); }
      }}>
        <span className="truncate">{value}</span><Icon name="chevronDown" className={`h-3.5 w-3.5 shrink-0 text-muted transition-transform ${open ? "rotate-180" : ""}`} />
      </button>
      {open ? <div id={`${id}-options`} role="listbox" aria-label={`${label} options`} className="absolute left-0 top-full z-50 mt-1.5 max-h-64 w-full min-w-0 overflow-auto rounded-xl border border-border bg-surface p-1.5 text-text shadow-xl shadow-black/25 ring-1 ring-white/5">
        {options.map((option, index) => <button key={option.id} id={`${id}-option-${index}`} type="button" role="option" aria-selected={option.id === selectedId} className={`flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-sm transition-colors ${index === active ? "bg-surface-2" : "hover:bg-surface-2"}`} onMouseEnter={() => setActive(index)} onClick={() => { onSelect(option.id); onOpenChange(false); }} onKeyDown={(event) => { if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); move(event.key === "ArrowDown" ? 1 : -1); } }}>
          <span className="min-w-0 flex-1 truncate">{option.label}</span>{option.id === selectedId ? <Icon name="check" className="h-3.5 w-3.5 shrink-0 text-accent" /> : null}
        </button>)}
        {options.length === 0 ? <p className="px-2.5 py-2 text-xs text-muted">No {label.toLowerCase()} available</p> : null}
      </div> : null}
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
