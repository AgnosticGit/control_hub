"use client";

import Link from "next/link";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Fragment, useState, type FormEvent } from "react";
import { Icon, Logo } from "@/components/ui/Icon";
import { api, apiSentence, unwrap, unwrapEmpty, type Schemas } from "@/lib/api";
import { useLogout, useSession } from "@/lib/session";
import { assignableRole, groupCatalog, memberLock, roleLock, toggle, type Actor, type CatalogEntry } from "@/components/settings/matrix";
import {
  accountLock,
  ADMIN_TAB_LABELS,
  adminTabs,
  auditActionLabel,
  canAdmin,
  projectDeleteLock,
  SYSTEM_MODEL,
  systemActor,
  type AdminTab,
  type SystemAccess,
} from "./admin-model";

type Project = Schemas["AdminProjectDto"];
type Account = Schemas["AdminUserDto"];
type Member = Schemas["MemberDto"];
type Role = Schemas["RoleDto"];

const PROJECT_ROLES = ["owner", "admin", "operator", "viewer"] as const;

function when(iso: string) {
  return iso ? new Date(iso).toLocaleString() : "—";
}

function ErrorLine({ error }: { error: unknown }) {
  if (!error) return null;
  return (
    <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
      {apiSentence(error)}
    </p>
  );
}

export function AdminPage() {
  const session = useSession();
  const logout = useLogout();
  const system = session.data?.system ?? null;
  const tabs = adminTabs(system);
  const [picked, setPicked] = useState<AdminTab>("overview");
  const tab = tabs.includes(picked) ? picked : "overview";
  const user = session.data?.user;

  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  if (!system || !user || tabs.length === 0) {
    return (
      <main className="flex min-h-screen items-center justify-center p-8">
        <div className="card max-w-md p-6 text-sm">
          <p>This page does not exist or you do not have access to it.</p>
          <Link href="/app" className="btn mt-4">
            <Icon name="arrowLeft" className="h-3.5 w-3.5" />
            Back to projects
          </Link>
        </div>
      </main>
    );
  }
  const actor = systemActor(user.id, system);

  return (
    <div className="min-h-screen">
      <header className="flex h-14 items-center gap-3 border-b border-border bg-bg-2/60 px-5">
        <Link href="/app" className="flex items-center gap-2.5 text-[17px] font-semibold tracking-tight">
          <Logo />
          karbird
        </Link>
        <span className="badge border-accent/50 bg-accent/10 text-accent">Admin</span>
        <span className="ml-auto text-xs text-muted">
          {user.email} · <span className="text-text">{system.role_name}</span>
        </span>
        <Link href="/app" className="btn btn-sm">
          <Icon name="arrowLeft" className="h-3.5 w-3.5" />
          Back to projects
        </Link>
        <button type="button" onClick={() => void logout()} className="btn btn-icon btn-sm" aria-label="Sign out" title="Sign out">
          <Icon name="logout" className="h-3.5 w-3.5" />
        </button>
      </header>
      <main className="mx-auto max-w-6xl space-y-5 p-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">karbird administration</h1>
          <p className="mt-1 max-w-3xl text-sm text-muted">
            Every project, account and administrator of this karbird. Administrator roles work like project roles: you can only
            grant what you have yourself, only a Super Admin changes a Super Admin, and one Super Admin always remains. Every change is
            recorded in the audit log.
          </p>
        </div>
        <div role="tablist" aria-label="Administration" className="flex gap-1 border-b border-border">
          {tabs.map((t) => (
            <button
              key={t}
              type="button"
              role="tab"
              aria-selected={tab === t}
              onClick={() => setPicked(t)}
              className={`-mb-px border-b-2 px-4 py-2 text-sm transition-colors ${
                tab === t ? "border-accent text-text" : "border-transparent text-muted hover:text-text"
              }`}
            >
              {ADMIN_TAB_LABELS[t]}
            </button>
          ))}
        </div>
        {tab === "overview" ? <OverviewTab /> : null}
        {tab === "projects" ? <ProjectsTab actor={actor} /> : null}
        {tab === "users" ? <AccountsTab actor={actor} system={system} /> : null}
        {tab === "admins" ? <AdminsTab actor={actor} /> : null}
        {tab === "roles" ? <RolesTab actor={actor} /> : null}
        {tab === "audit" ? <AuditTab system={system} /> : null}
      </main>
    </div>
  );
}

function OverviewTab() {
  const overview = useQuery({ queryKey: ["admin", "overview"], queryFn: async () => unwrap(await api.GET("/api/v1/admin/overview")) });
  const o = overview.data;
  const cards: Array<[string, number | undefined]> = [
    ["Projects", o?.projects],
    ["Environments", o?.environments],
    ["Accounts", o?.users],
    ["Disabled accounts", o?.disabled_users],
    ["Active sessions", o?.active_sessions],
    ["Administrators", o?.administrators],
  ];
  return (
    <div className="space-y-3">
      <ErrorLine error={overview.error} />
      <dl className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {cards.map(([label, value]) => (
          <div key={label} className="card p-4">
            <dt className="label">{label}</dt>
            <dd className="mt-1 text-2xl font-semibold tabular-nums">{value ?? "…"}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/** Refetches the admin data and the session; `deletedProject`'s member list is dropped instead of
 *  refetched (it would only answer 404). */
function useAdminInvalidate() {
  const qc = useQueryClient();
  return (deletedProject?: string) => {
    if (deletedProject) qc.removeQueries({ queryKey: ["admin", "project-members", deletedProject] });
    return Promise.all([
      qc.invalidateQueries({
        queryKey: ["admin"],
        predicate: (q) => !(deletedProject && q.queryKey[1] === "project-members" && q.queryKey[2] === deletedProject),
      }),
      qc.invalidateQueries({ queryKey: ["session"] }),
    ]);
  };
}

function ProjectsTab({ actor }: { actor: Actor }) {
  const invalidate = useAdminInvalidate();
  const projects = useQuery({ queryKey: ["admin", "projects"], queryFn: async () => unwrap(await api.GET("/api/v1/admin/projects")) });
  const canManage = actor.permissions.includes("system.projects.manage");
  const [name, setName] = useState("");
  const [owner, setOwner] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const path = (id: string) => ({ params: { path: { project_id: id } } });

  const create = useMutation({
    mutationFn: async () =>
      unwrap(await api.POST("/api/v1/admin/projects", { body: { name, owner_email: owner.trim() ? owner.trim() : null } })),
    onSuccess: () => {
      setName("");
      setOwner("");
    },
    onSettled: () => invalidate(),
  });
  const rename = useMutation({
    mutationFn: async ({ id, name }: { id: string; name: string }) =>
      unwrap(await api.PATCH("/api/v1/admin/projects/{project_id}", { ...path(id), body: { name } })),
    onSettled: () => invalidate(),
  });
  const addEnv = useMutation({
    mutationFn: async ({ id, name }: { id: string; name: string }) =>
      unwrap(await api.POST("/api/v1/admin/projects/{project_id}/environments", { ...path(id), body: { name } })),
    onSettled: () => invalidate(),
  });
  const remove = useMutation({
    mutationFn: async (id: string) => unwrapEmpty(await api.DELETE("/api/v1/admin/projects/{project_id}", path(id))),
    onSuccess: (_data, id) => setOpen((o) => (o === id ? null : o)),
    onSettled: (_data, error, id) => invalidate(error ? undefined : id),
  });
  const error = projects.error ?? create.error ?? rename.error ?? addEnv.error ?? remove.error;

  const onCreate = (e: FormEvent) => {
    e.preventDefault();
    create.mutate();
  };

  return (
    <div className="space-y-4">
      {canManage ? (
        <form onSubmit={onCreate} aria-label="New project" className="card flex flex-wrap items-end gap-3 p-4 text-sm">
          <label className="flex flex-col gap-1.5">
            <span className="label">Project name</span>
            <input required maxLength={100} value={name} onChange={(e) => setName(e.target.value)} className="input h-9 w-64" />
          </label>
          <label className="flex flex-col gap-1.5">
            <span className="label">Owner email (optional)</span>
            <input type="email" value={owner} onChange={(e) => setOwner(e.target.value)} className="input h-9 w-64" placeholder="you by default" />
          </label>
          <button type="submit" disabled={create.isPending || !name.trim()} className="btn btn-primary h-9 px-4">
            <Icon name="plus" className="h-4 w-4" />
            Create project
          </button>
          <p className="basis-full text-xs text-muted">The project starts with one environment, “production”. The owner must already have an account.</p>
        </form>
      ) : null}
      <ErrorLine error={error} />
      <div className="card overflow-hidden">
        <table className="w-full text-left text-sm" aria-label="All projects">
          <thead className="table-head">
            <tr>
              <th className="px-4 py-2">Project</th>
              <th className="px-2">Owners</th>
              <th className="px-2">Members</th>
              <th className="px-2">Environments</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {(projects.data ?? []).map((p) => {
              const deleteLock = projectDeleteLock(actor, p);
              const expanded = open === p.id;
              return (
                <Fragment key={`${p.tenant_id}/${p.id}`}>
                  <tr className="border-t border-border">
                    <td className="px-4 py-2.5">
                      <div className="font-medium">{p.name}</div>
                      <div className="text-xs text-muted">
                        {p.id} · created {when(p.created_at)}
                      </div>
                    </td>
                    <td className="px-2 text-xs text-muted">{p.owners.join(", ") || "—"}</td>
                    <td className="px-2 tabular-nums">{p.members}</td>
                    <td className="px-2">
                      <div className="flex flex-wrap gap-1.5">
                        {p.environments.map((e) => (
                          <span key={e.id} className="badge border-border-strong text-muted">
                            {e.name}
                            {e.is_demo ? " · DEMO" : ""}
                          </span>
                        ))}
                      </div>
                    </td>
                    <td className="space-x-2 px-3 text-right whitespace-nowrap">
                      <button type="button" className="btn btn-sm" aria-expanded={expanded} onClick={() => setOpen(expanded ? null : p.id)}>
                        {expanded ? "Hide members" : "Members"}
                      </button>
                      {canManage ? (
                        <>
                          <button
                            type="button"
                            className="btn btn-sm"
                            aria-label={`Rename ${p.name}`}
                            disabled={rename.isPending}
                            onClick={() => {
                              const next = window.prompt("New project name:", p.name);
                              if (next && next.trim() && next.trim() !== p.name) rename.mutate({ id: p.id, name: next.trim() });
                            }}
                          >
                            Rename
                          </button>
                          <button
                            type="button"
                            className="btn btn-sm"
                            aria-label={`Add environment to ${p.name}`}
                            disabled={addEnv.isPending}
                            onClick={() => {
                              const next = window.prompt(`New environment of ${p.name}:`, "staging");
                              if (next && next.trim()) addEnv.mutate({ id: p.id, name: next.trim() });
                            }}
                          >
                            Add environment
                          </button>
                        </>
                      ) : null}
                      {canManage ? (
                        <button
                          type="button"
                          className="btn btn-sm btn-danger"
                          aria-label={`Delete ${p.name}`}
                          disabled={deleteLock !== null || remove.isPending}
                          title={deleteLock ?? undefined}
                          onClick={() => {
                            const typed = window.prompt(
                              `Delete “${p.name}” with all its environments, members, roles, tokens and plans? This cannot be undone. Type the project name to confirm:`,
                            );
                            if (typed === p.name) remove.mutate(p.id);
                          }}
                        >
                          Delete
                        </button>
                      ) : null}
                    </td>
                  </tr>
                  {expanded ? (
                    <tr className="bg-surface-2/50">
                      <td colSpan={5} className="px-4 py-3">
                        <ProjectMembers project={p} canManage={canManage} />
                      </td>
                    </tr>
                  ) : null}
                </Fragment>
              );
            })}
            {projects.data?.length === 0 ? (
              <tr className="border-t border-border">
                <td colSpan={5} className="px-4 py-3 text-muted">
                  No projects yet.
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function ProjectMembers({ project, canManage }: { project: Project; canManage: boolean }) {
  const invalidate = useAdminInvalidate();
  const path = { project_id: project.id };
  const members = useQuery({
    queryKey: ["admin", "project-members", project.id],
    queryFn: async () => unwrap(await api.GET("/api/v1/admin/projects/{project_id}/members", { params: { path } })),
  });
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<string>("viewer");
  const put = useMutation({
    mutationFn: async (body: Schemas["AdminProjectMemberRequest"]) =>
      unwrap(await api.PUT("/api/v1/admin/projects/{project_id}/members", { params: { path }, body })),
    onSuccess: () => setEmail(""),
    onSettled: () => invalidate(),
  });
  const remove = useMutation({
    mutationFn: async (userId: string) =>
      unwrapEmpty(await api.DELETE("/api/v1/admin/projects/{project_id}/members/{user_id}", { params: { path: { ...path, user_id: userId } } })),
    onSettled: () => invalidate(),
  });
  const onAdd = (e: FormEvent) => {
    e.preventDefault();
    put.mutate({ email, role });
  };

  return (
    <div className="space-y-3">
      <ErrorLine error={members.error ?? put.error ?? remove.error} />
      <table className="w-full text-left text-sm" aria-label={`Members of ${project.name}`}>
        <tbody>
          {(members.data ?? []).map((m) => {
            const name = m.display_name || m.email;
            return (
              <tr key={m.user_id} className="border-t border-border first:border-t-0">
                <td className="py-1.5">
                  <span className="font-medium">{name}</span> <span className="text-xs text-muted">{m.email}</span>
                </td>
                <td className="px-2">
                  {canManage ? (
                    <select
                      aria-label={`Project role for ${name}`}
                      className="select h-8 text-sm"
                      value={PROJECT_ROLES.includes(m.role as (typeof PROJECT_ROLES)[number]) ? m.role : ""}
                      disabled={put.isPending}
                      onChange={(e) => put.mutate({ email: m.email, role: e.target.value })}
                    >
                      {PROJECT_ROLES.includes(m.role as (typeof PROJECT_ROLES)[number]) ? null : (
                        <option value="" disabled>
                          {m.role_name}
                        </option>
                      )}
                      {PROJECT_ROLES.map((r) => (
                        <option key={r} value={r}>
                          {r.charAt(0).toUpperCase() + r.slice(1)}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <span className="badge border-border-strong text-muted">{m.role_name}</span>
                  )}
                </td>
                <td className="text-right">
                  {canManage ? (
                    <button
                      type="button"
                      className="btn btn-sm btn-danger"
                      aria-label={`Remove ${name} from ${project.name}`}
                      disabled={remove.isPending}
                      onClick={() => {
                        if (window.confirm(`Remove ${name} from ${project.name}?`)) remove.mutate(m.user_id);
                      }}
                    >
                      Remove
                    </button>
                  ) : null}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {canManage ? (
        <form onSubmit={onAdd} aria-label={`Add member to ${project.name}`} className="flex flex-wrap items-end gap-2 text-sm">
          <label className="flex flex-col gap-1">
            <span className="label">Account email</span>
            <input type="email" required value={email} onChange={(e) => setEmail(e.target.value)} className="input h-8 w-64" />
          </label>
          <label className="flex flex-col gap-1">
            <span className="label">Role</span>
            <select value={role} onChange={(e) => setRole(e.target.value)} className="select h-8 text-sm">
              {PROJECT_ROLES.map((r) => (
                <option key={r} value={r}>
                  {r.charAt(0).toUpperCase() + r.slice(1)}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" className="btn btn-sm h-8" disabled={put.isPending}>
            <Icon name="plus" className="h-3.5 w-3.5" />
            Add member
          </button>
        </form>
      ) : null}
    </div>
  );
}

function AccountsTab({ actor, system }: { actor: Actor; system: SystemAccess }) {
  const invalidate = useAdminInvalidate();
  const users = useQuery({ queryKey: ["admin", "users"], queryFn: async () => unwrap(await api.GET("/api/v1/admin/users")) });
  const admins = useQuery({
    queryKey: ["admin", "members"],
    enabled: canAdmin(system, "system.team.view"),
    queryFn: async () => unwrap(await api.GET("/api/v1/admin/members")),
  });
  const [filter, setFilter] = useState("");
  const path = (id: string) => ({ params: { path: { user_id: id } } });
  const disable = useMutation({
    mutationFn: async ({ id, on }: { id: string; on: boolean }) =>
      unwrap(await (on ? api.POST("/api/v1/admin/users/{user_id}/disable", path(id)) : api.POST("/api/v1/admin/users/{user_id}/enable", path(id)))),
    onSettled: () => invalidate(),
  });
  const signOut = useMutation({
    mutationFn: async (id: string) => unwrap(await api.POST("/api/v1/admin/users/{user_id}/sessions/revoke", path(id))),
    onSettled: () => invalidate(),
  });
  const needle = filter.trim().toLowerCase();
  const shown = (users.data ?? []).filter((u) => !needle || u.email.toLowerCase().includes(needle) || u.display_name.toLowerCase().includes(needle));
  const adminOf = (u: Account) => admins.data?.find((m) => m.user_id === u.id);

  return (
    <div className="space-y-3">
      <label className="flex max-w-sm flex-col gap-1.5 text-sm">
        <span className="label">Find account</span>
        <input type="search" value={filter} onChange={(e) => setFilter(e.target.value)} className="input h-9" placeholder="Email or name" />
      </label>
      <ErrorLine error={users.error ?? disable.error ?? signOut.error} />
      <div className="card overflow-hidden">
        <table className="w-full text-left text-sm" aria-label="Accounts">
          <thead className="table-head">
            <tr>
              <th className="px-4 py-2">Account</th>
              <th className="px-2">Projects</th>
              <th className="px-2">Sessions</th>
              <th className="px-2">Administrator</th>
              <th className="px-2">Status</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {shown.map((u) => {
              const lock = accountLock(actor, u, adminOf(u));
              const name = u.display_name || u.email;
              return (
                <tr key={u.id} className="border-t border-border">
                  <td className="px-4 py-2.5">
                    <div className="font-medium">
                      {name}
                      {u.id === actor.userId ? <span className="ml-2 text-xs text-muted">(you)</span> : null}
                    </div>
                    <div className="text-xs text-muted">
                      {u.email} · joined {when(u.created_at)}
                    </div>
                  </td>
                  <td className="px-2 tabular-nums">{u.projects}</td>
                  <td className="px-2 tabular-nums">{u.active_sessions}</td>
                  <td className="px-2 text-xs">{u.system_role ?? <span className="text-muted">—</span>}</td>
                  <td className="px-2">
                    {u.disabled ? (
                      <span className="badge border-error/50 bg-error/10 text-error">Disabled</span>
                    ) : (
                      <span className="badge border-ok/50 bg-ok/10 text-ok">Active</span>
                    )}
                  </td>
                  <td className="space-x-2 px-3 text-right whitespace-nowrap">
                    {canAdmin(system, "system.users.manage") ? (
                      <>
                        <button
                          type="button"
                          className="btn btn-sm"
                          aria-label={`End sessions of ${name}`}
                          disabled={lock !== null || signOut.isPending || u.active_sessions === 0}
                          title={lock ?? undefined}
                          onClick={() => signOut.mutate(u.id)}
                        >
                          End sessions
                        </button>
                        <button
                          type="button"
                          className={`btn btn-sm ${u.disabled ? "" : "btn-danger"}`}
                          aria-label={`${u.disabled ? "Enable" : "Disable"} ${name}`}
                          disabled={lock !== null || disable.isPending}
                          title={lock ?? undefined}
                          onClick={() => {
                            if (u.disabled || window.confirm(`Disable ${name}? They are signed out at once and cannot sign in.`)) {
                              disable.mutate({ id: u.id, on: !u.disabled });
                            }
                          }}
                        >
                          {u.disabled ? "Enable" : "Disable"}
                        </button>
                      </>
                    ) : null}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function useSystemCatalog() {
  return useQuery({
    queryKey: ["admin", "permissions"],
    staleTime: Infinity,
    queryFn: async () => unwrap(await api.GET("/api/v1/admin/permissions")),
  });
}

function useSystemRoles() {
  return useQuery({ queryKey: ["admin", "roles"], queryFn: async () => unwrap(await api.GET("/api/v1/admin/roles")) });
}

function AdminsTab({ actor }: { actor: Actor }) {
  const invalidate = useAdminInvalidate();
  const catalog = useSystemCatalog();
  const roles = useSystemRoles();
  const members = useQuery({ queryKey: ["admin", "members"], queryFn: async () => unwrap(await api.GET("/api/v1/admin/members")) });
  const [open, setOpen] = useState<string | null>(null);
  const [email, setEmail] = useState("");
  const [roleId, setRoleId] = useState("");
  const canManage = actor.permissions.includes("system.team.manage");

  const add = useMutation({
    mutationFn: async (body: Schemas["AdminMemberCreateRequest"]) => unwrap(await api.POST("/api/v1/admin/members", { body })),
    onSuccess: () => setEmail(""),
    onSettled: () => invalidate(),
  });
  const update = useMutation({
    mutationFn: async ({ id, body }: { id: string; body: Schemas["MemberUpdateRequest"] }) =>
      unwrap(await api.PATCH("/api/v1/admin/members/{user_id}", { params: { path: { user_id: id } }, body })),
    onSettled: () => invalidate(),
  });
  const remove = useMutation({
    mutationFn: async (id: string) => unwrapEmpty(await api.DELETE("/api/v1/admin/members/{user_id}", { params: { path: { user_id: id } } })),
    onSettled: () => invalidate(),
  });
  const error = catalog.error ?? roles.error ?? members.error ?? add.error ?? update.error ?? remove.error;
  const assignable = (roles.data ?? []).filter((r) => assignableRole(actor, r, SYSTEM_MODEL));
  const chosenRole = roleId || assignable.find((r) => r.builtin === "auditor")?.id || assignable[0]?.id || "";

  const onAdd = (e: FormEvent) => {
    e.preventDefault();
    if (chosenRole) add.mutate({ email, role_id: chosenRole });
  };

  return (
    <div className="space-y-4">
      {canManage ? (
        <form onSubmit={onAdd} aria-label="Add administrator" className="card flex flex-wrap items-end gap-3 p-4 text-sm">
          <label className="flex flex-col gap-1.5">
            <span className="label">Account email</span>
            <input type="email" required value={email} onChange={(e) => setEmail(e.target.value)} className="input h-9 w-64" />
          </label>
          <label className="flex flex-col gap-1.5">
            <span className="label">Administrator role</span>
            <select value={chosenRole} onChange={(e) => setRoleId(e.target.value)} className="select h-9 min-w-48 text-sm">
              {assignable.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" disabled={add.isPending || !chosenRole} className="btn btn-primary h-9 px-4">
            <Icon name="plus" className="h-4 w-4" />
            Add administrator
          </button>
          <p className="basis-full text-xs text-muted">The account must already exist. You can only grant roles within your own permissions.</p>
        </form>
      ) : null}
      <ErrorLine error={error} />
      {catalog.data && roles.data && members.data ? (
        <div className="card overflow-hidden">
          <table className="w-full text-left text-sm" aria-label="Administrators">
            <thead className="table-head">
              <tr>
                <th className="px-4 py-2">Administrator</th>
                <th className="px-2">Role</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {members.data.map((m) => (
                <AdminRow
                  key={m.user_id}
                  member={m}
                  actor={actor}
                  roles={roles.data}
                  catalog={catalog.data}
                  expanded={open === m.user_id}
                  onToggle={() => setOpen(open === m.user_id ? null : m.user_id)}
                  busy={update.isPending || remove.isPending}
                  onChange={(body) => update.mutate({ id: m.user_id, body })}
                  onRemove={() => remove.mutate(m.user_id)}
                />
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="text-sm text-muted">Loading…</p>
      )}
    </div>
  );
}

function AdminRow({
  member: m,
  actor,
  roles,
  catalog,
  expanded,
  onToggle,
  busy,
  onChange,
  onRemove,
}: {
  member: Member;
  actor: Actor;
  roles: Role[];
  catalog: CatalogEntry[];
  expanded: boolean;
  onToggle: () => void;
  busy: boolean;
  onChange: (body: Schemas["MemberUpdateRequest"]) => void;
  onRemove: () => void;
}) {
  const lock = memberLock(actor, m, SYSTEM_MODEL);
  const name = m.display_name || m.email;
  const roleName = (id: string | null | undefined) => roles.find((r) => r.id === id)?.name;
  return (
    <>
      <tr className="border-t border-border">
        <td className="px-4 py-2.5">
          <div className="font-medium">
            {name}
            {m.user_id === actor.userId ? <span className="ml-2 text-xs text-muted">(you)</span> : null}
          </div>
          <div className="text-xs text-muted">{m.email}</div>
        </td>
        <td className="px-2">
          <select
            aria-label={`Administrator role for ${name}`}
            className="select h-8 min-w-48 text-sm"
            value={m.role_id ?? ""}
            disabled={lock !== null || busy}
            title={lock ?? undefined}
            onChange={(e) => onChange({ role_id: e.target.value })}
          >
            {m.role_id ? null : (
              <option value="" disabled>
                Individual{m.based_on && roleName(m.based_on) ? `, based on ${roleName(m.based_on)}` : ""}
              </option>
            )}
            {roles.map((r) => (
              <option key={r.id} value={r.id} disabled={!assignableRole(actor, r, SYSTEM_MODEL)}>
                {r.name}
              </option>
            ))}
          </select>
        </td>
        <td className="space-x-2 px-3 text-right whitespace-nowrap">
          <button type="button" className="btn btn-sm" aria-expanded={expanded} onClick={onToggle}>
            {expanded ? "Hide permissions" : "Permissions"}
          </button>
          <button
            type="button"
            className="btn btn-sm btn-danger"
            aria-label={`Remove administrator ${name}`}
            disabled={lock !== null || busy}
            title={lock ?? undefined}
            onClick={() => {
              if (window.confirm(`Remove ${name}'s administrator access? Their project memberships stay.`)) onRemove();
            }}
          >
            Remove
          </button>
        </td>
      </tr>
      {expanded ? (
        <tr className="bg-surface-2/50">
          <td colSpan={3} className="px-4 py-3">
            {lock ? <p className="mb-3 text-xs text-muted">{lock}</p> : null}
            <fieldset aria-label={`Administrator permissions of ${name}`} className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
              {groupCatalog(catalog).map((g) => (
                <div key={g.group}>
                  <div className="label mb-1.5">{g.group}</div>
                  {g.items.map((p) => {
                    const outOfReach = !actor.permissions.includes(p.id);
                    return (
                      <label key={p.id} className="flex items-start gap-2 py-0.5 text-sm">
                        <input
                          type="checkbox"
                          className="mt-1"
                          checked={m.permissions.includes(p.id)}
                          disabled={lock !== null || outOfReach || busy}
                          title={outOfReach ? "You do not have this permission yourself." : undefined}
                          onChange={(e) =>
                            onChange({
                              permissions: toggle(m.permissions, p.id, e.target.checked, SYSTEM_MODEL),
                              based_on: m.role_id ?? m.based_on ?? null,
                            })
                          }
                        />
                        {p.label}
                      </label>
                    );
                  })}
                </div>
              ))}
            </fieldset>
          </td>
        </tr>
      ) : null}
    </>
  );
}

function RolesTab({ actor }: { actor: Actor }) {
  const invalidate = useAdminInvalidate();
  const catalog = useSystemCatalog();
  const roles = useSystemRoles();
  const [name, setName] = useState("");
  const canManage = actor.permissions.includes("system.roles.manage");
  const create = useMutation({
    mutationFn: async () => unwrap(await api.POST("/api/v1/admin/roles", { body: { name, permissions: [] } })),
    onSuccess: () => setName(""),
    onSettled: () => invalidate(),
  });
  const save = useMutation({
    mutationFn: async ({ role, name, permissions }: { role: Role; name: string; permissions: string[] }) =>
      unwrap(await api.PATCH("/api/v1/admin/roles/{role_id}", { params: { path: { role_id: role.id } }, body: { name, permissions } })),
    onSettled: () => invalidate(),
  });
  const remove = useMutation({
    mutationFn: async (role: Role) => unwrapEmpty(await api.DELETE("/api/v1/admin/roles/{role_id}", { params: { path: { role_id: role.id } } })),
    onSettled: () => invalidate(),
  });
  const error = catalog.error ?? roles.error ?? create.error ?? save.error ?? remove.error;
  const onCreate = (e: FormEvent) => {
    e.preventDefault();
    create.mutate();
  };

  return (
    <div className="space-y-4">
      {canManage ? (
        <form onSubmit={onCreate} aria-label="New administrator role" className="card flex flex-wrap items-end gap-3 p-4 text-sm">
          <label className="flex flex-col gap-1.5">
            <span className="label">Role name</span>
            <input required maxLength={64} value={name} onChange={(e) => setName(e.target.value)} className="input h-9 w-64" placeholder="e.g. Support" />
          </label>
          <button type="submit" disabled={create.isPending} className="btn btn-primary h-9 px-4">
            <Icon name="plus" className="h-4 w-4" />
            Create role
          </button>
          <p className="basis-full text-xs text-muted">A new role starts without permissions; tick them below.</p>
        </form>
      ) : null}
      <ErrorLine error={error} />
      {catalog.data && roles.data ? (
        <div className="card overflow-x-auto">
          <table className="w-full text-left text-sm" aria-label="Administrator roles">
            <thead className="table-head">
              <tr>
                <th className="px-4 py-2">Permission</th>
                {roles.data.map((r) => {
                  const lock = roleLock(actor, r, SYSTEM_MODEL);
                  return (
                    <th key={r.id} scope="col" className="min-w-32 px-2 py-2 text-center align-top">
                      <div className="font-medium text-text normal-case">{r.name}</div>
                      <div className="text-[11px] font-normal normal-case text-muted">
                        {r.builtin ? "built-in" : "custom"} · {r.holders} {r.holders === 1 ? "holder" : "holders"}
                      </div>
                      {r.builtin || lock ? null : (
                        <div className="mt-1 flex justify-center gap-1">
                          <button
                            type="button"
                            className="btn btn-sm"
                            aria-label={`Rename ${r.name}`}
                            onClick={() => {
                              const next = window.prompt("New role name:", r.name);
                              if (next && next.trim() && next.trim() !== r.name) save.mutate({ role: r, name: next.trim(), permissions: r.permissions });
                            }}
                          >
                            Rename
                          </button>
                          <button
                            type="button"
                            className="btn btn-sm btn-danger"
                            aria-label={`Delete ${r.name}`}
                            disabled={r.holders > 0 || remove.isPending}
                            title={r.holders > 0 ? "Give its holders another role first." : undefined}
                            onClick={() => {
                              if (window.confirm(`Delete the role “${r.name}”?`)) remove.mutate(r);
                            }}
                          >
                            Delete
                          </button>
                        </div>
                      )}
                    </th>
                  );
                })}
              </tr>
            </thead>
            <tbody>
              {groupCatalog(catalog.data).map((g) => (
                <Fragment key={g.group}>
                  <tr className="border-t border-border bg-surface-2/40">
                    <th colSpan={roles.data.length + 1} scope="colgroup" className="px-4 py-1.5 text-xs font-medium text-muted">
                      {g.group}
                    </th>
                  </tr>
                  {g.items.map((p) => (
                    <tr key={p.id} className="border-t border-border">
                      <th scope="row" className="px-4 py-2 font-normal">
                        {p.label}
                      </th>
                      {roles.data.map((r) => {
                        const lock = roleLock(actor, r, SYSTEM_MODEL);
                        const outOfReach = !actor.permissions.includes(p.id);
                        return (
                          <td key={r.id} className="px-2 text-center">
                            <input
                              type="checkbox"
                              aria-label={`${p.label} for ${r.name}`}
                              checked={r.permissions.includes(p.id)}
                              disabled={lock !== null || outOfReach || save.isPending}
                              title={lock ?? (outOfReach ? "You do not have this permission yourself." : undefined)}
                              onChange={(e) =>
                                save.mutate({ role: r, name: r.name, permissions: toggle(r.permissions, p.id, e.target.checked, SYSTEM_MODEL) })
                              }
                            />
                          </td>
                        );
                      })}
                    </tr>
                  ))}
                </Fragment>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="text-sm text-muted">Loading…</p>
      )}
    </div>
  );
}

function AuditTab({ system }: { system: SystemAccess }) {
  const [project, setProject] = useState("");
  const projects = useQuery({
    queryKey: ["admin", "projects"],
    enabled: canAdmin(system, "system.projects.view"),
    queryFn: async () => unwrap(await api.GET("/api/v1/admin/projects")),
  });
  const audit = useQuery({
    queryKey: ["admin", "audit", project],
    queryFn: async () => unwrap(await api.GET("/api/v1/admin/audit", { params: { query: { project_id: project || undefined } } })),
  });
  const projectName = (id: string | null | undefined) => (id ? (projects.data?.find((p) => p.id === id)?.name ?? id) : "karbird");

  return (
    <div className="space-y-3">
      {projects.data ? (
        <label className="flex max-w-sm flex-col gap-1.5 text-sm">
          <span className="label">Project</span>
          <select value={project} onChange={(e) => setProject(e.target.value)} className="select h-9 text-sm">
            <option value="">All projects and karbird</option>
            {projects.data.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      <ErrorLine error={audit.error} />
      <div className="card overflow-hidden">
        <table className="w-full text-left text-sm" aria-label="Audit log">
          <thead className="table-head">
            <tr>
              <th className="px-4 py-2">When</th>
              <th className="px-2">Who</th>
              <th className="px-2">What</th>
              <th className="px-2">Where</th>
              <th className="px-2">Outcome</th>
            </tr>
          </thead>
          <tbody>
            {(audit.data ?? []).map((a) => (
              <tr key={a.id} className="border-t border-border align-top">
                <td className="px-4 py-2 text-xs whitespace-nowrap text-muted">{when(a.occurred_at)}</td>
                <td className="px-2 py-2 text-xs">{a.actor_email ?? `${a.actor_kind} ${a.actor_id}`}</td>
                <td className="px-2 py-2">
                  <div>{auditActionLabel(a.action)}</div>
                  {a.reason ? <div className="text-xs text-muted">{a.reason}</div> : null}
                </td>
                <td className="px-2 py-2 text-xs text-muted">{projectName(a.project_id)}</td>
                <td className="px-2 py-2">
                  <span
                    className={`badge ${a.outcome === "succeeded" ? "border-ok/50 bg-ok/10 text-ok" : a.outcome === "denied" ? "border-error/50 bg-error/10 text-error" : "border-border-strong text-muted"}`}
                  >
                    {a.outcome}
                  </span>
                </td>
              </tr>
            ))}
            {audit.data?.length === 0 ? (
              <tr className="border-t border-border">
                <td colSpan={5} className="px-4 py-3 text-muted">
                  Nothing recorded yet.
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
    </div>
  );
}
