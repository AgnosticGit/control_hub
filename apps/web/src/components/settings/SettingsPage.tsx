"use client";

import Link from "next/link";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Fragment, useState, type FormEvent } from "react";
import { AppShell } from "@/components/shell/AppShell";
import { Icon } from "@/components/ui/Icon";
import { api, ApiRequestError, unwrap, unwrapEmpty, type Schemas } from "@/lib/api";
import { can, type PermissionId } from "@/lib/permissions";
import { useSession } from "@/lib/session";
import { InvitesTab } from "./InvitesTab";
import { assignableRole, groupCatalog, memberAfter, memberLock, roleLock, toggle, type Actor, type CatalogEntry } from "./matrix";

type Role = Schemas["RoleDto"];
type Member = Schemas["MemberDto"];

const ROLES = "/api/v1/projects/{project_id}/roles" as const;
const MEMBERS = "/api/v1/projects/{project_id}/members" as const;

function message(e: unknown) {
  return e instanceof ApiRequestError ? e.message : e instanceof Error ? e.message : String(e);
}

function when(iso: string) {
  return iso ? new Date(iso).toLocaleDateString() : "—";
}

type Tab = "members" | "roles" | "invites";

export function SettingsPage({ projectId, environmentId }: { projectId: string; environmentId: string }) {
  const session = useSession();
  const [tab, setTab] = useState<Tab>("members");
  const project = session.data?.projects.find((p) => p.id === projectId);
  const env = project?.environments.find((e) => e.id === environmentId);
  const canView = can(project, "members.view");
  const path = { project_id: projectId };

  const catalog = useQuery({
    queryKey: ["permissions"],
    enabled: canView,
    staleTime: Infinity,
    queryFn: async () => unwrap(await api.GET("/api/v1/permissions")),
  });
  const roles = useQuery({
    queryKey: ["roles", projectId],
    enabled: canView,
    queryFn: async () => unwrap(await api.GET(ROLES, { params: { path } })),
  });
  const members = useQuery({
    queryKey: ["members", projectId],
    enabled: canView,
    queryFn: async () => unwrap(await api.GET(MEMBERS, { params: { path } })),
  });

  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  if (!env || !project) {
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

  const actor: Actor = { userId: session.data?.user.id ?? "", permissions: project.permissions, isOwner: project.role === "owner" };
  const error = catalog.error ?? roles.error ?? members.error;

  return (
    <AppShell
      projectId={projectId}
      environmentId={environmentId}
      active="settings"
      top={
        <>
          <span className="card-title-icon">
            <Icon name="users" className="h-3.5 w-3.5" />
          </span>
          <span className="text-sm text-muted">{project.name}</span>
        </>
      }
    >
      <main className="h-full overflow-auto">
        <div className="mx-auto max-w-6xl space-y-5 p-6">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">Members &amp; roles</h1>
            <p className="mt-1 max-w-3xl text-sm text-muted">
              Who can do what in <span className="text-text">{project.name}</span>. Give each member a role, or fine-tune an
              individual set of permissions. Roles you create here apply to this project only; editing a role changes it for
              everyone who has it. Your role: <span className="text-text">{project.role_name}</span>.
            </p>
          </div>

          {!canView ? (
            <p className="card p-4 text-sm text-muted">Your role in this project does not include viewing members.</p>
          ) : (
            <>
              <div role="tablist" aria-label="Members and roles" className="flex gap-1 border-b border-border">
                {(can(project, "members.manage") ? (["members", "roles", "invites"] as const) : (["members", "roles"] as const)).map((t) => (
                  <button
                    key={t}
                    type="button"
                    role="tab"
                    aria-selected={tab === t}
                    onClick={() => setTab(t)}
                    className={`-mb-px border-b-2 px-4 py-2 text-sm transition-colors ${
                      tab === t ? "border-accent text-text" : "border-transparent text-muted hover:text-text"
                    }`}
                  >
                    {t === "members" ? `Members${members.data ? ` (${members.data.length})` : ""}` : t === "roles" ? "Roles" : "Invite links"}
                  </button>
                ))}
              </div>

              {error ? (
                <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
                  {message(error)}
                </p>
              ) : null}

              {catalog.data && roles.data && members.data ? (
                tab === "members" ? (
                  <MembersTab projectId={projectId} actor={actor} catalog={catalog.data} roles={roles.data} members={members.data} />
                ) : tab === "roles" ? (
                  <RolesTab projectId={projectId} actor={actor} catalog={catalog.data} roles={roles.data} />
                ) : (
                  <InvitesTab projectId={projectId} actor={actor} roles={roles.data} />
                )
              ) : (
                <p className="text-sm text-muted">Loading…</p>
              )}
            </>
          )}
        </div>
      </main>
    </AppShell>
  );
}

function useInvalidate(projectId: string) {
  const qc = useQueryClient();
  return () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: ["roles", projectId] }),
      qc.invalidateQueries({ queryKey: ["members", projectId] }),
      qc.invalidateQueries({ queryKey: ["session"] }),
    ]);
}

/** Unsaved edits shown at once (set in the event handler, so controlled inputs follow the click)
 *  until the refetch after the request replaces them; on error the saved value comes back. */
function useDrafts<T>() {
  const [drafts, setDrafts] = useState<ReadonlyMap<string, T>>(new Map());
  return {
    get: (id: string) => drafts.get(id),
    set: (id: string, value: T) => setDrafts((d) => new Map(d).set(id, value)),
    clear: (id: string) =>
      setDrafts((d) => {
        const next = new Map(d);
        next.delete(id);
        return next;
      }),
  };
}

function MembersTab({
  projectId,
  actor,
  catalog,
  roles,
  members,
}: {
  projectId: string;
  actor: Actor;
  catalog: CatalogEntry[];
  roles: Role[];
  members: Member[];
}) {
  const invalidate = useInvalidate(projectId);
  const drafts = useDrafts<Partial<Member>>();
  const [open, setOpen] = useState<string | null>(null);
  const update = useMutation({
    mutationFn: async ({ member, body }: { member: Member; body: Schemas["MemberUpdateRequest"] }) =>
      unwrap(
        await api.PATCH(`${MEMBERS}/{user_id}`, { params: { path: { project_id: projectId, user_id: member.user_id } }, body }),
      ),
    onSettled: async (_data, _error, { member }) => {
      await invalidate();
      drafts.clear(member.user_id);
    },
  });
  const change = (member: Member, body: Schemas["MemberUpdateRequest"]) => {
    drafts.set(member.user_id, memberAfter(body, roles));
    update.mutate({ member, body });
  };
  const remove = useMutation({
    mutationFn: async (member: Member) =>
      unwrapEmpty(await api.DELETE(`${MEMBERS}/{user_id}`, { params: { path: { project_id: projectId, user_id: member.user_id } } })),
    onSettled: invalidate,
  });
  const groups = groupCatalog(catalog);
  const roleName = (id: string | null | undefined) => roles.find((r) => r.id === id)?.name;
  const error = update.error ?? remove.error;

  return (
    <div className="space-y-3">
      {error ? (
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
          {message(error)}
        </p>
      ) : null}
      <div className="card overflow-hidden">
        <table className="w-full text-left text-sm" aria-label="Members">
          <thead className="table-head">
            <tr>
              <th className="px-4 py-2">Member</th>
              <th className="px-2">Role</th>
              <th className="px-2">Joined</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {members.map((saved) => {
              const m = { ...saved, ...drafts.get(saved.user_id) };
              const lock = memberLock(actor, m);
              const name = m.display_name || m.email;
              const expanded = open === m.user_id;
              return (
                <Fragment key={m.user_id}>
                  <tr className="border-t border-border">
                    <td className="px-4 py-2.5">
                      <div className="flex items-center gap-3">
                        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-accent/20 text-xs font-semibold text-accent">
                          {name.slice(0, 1).toUpperCase()}
                        </span>
                        <div className="min-w-0">
                          <div className="truncate font-medium">
                            {name}
                            {m.user_id === actor.userId ? <span className="ml-2 text-xs text-muted">(you)</span> : null}
                          </div>
                          <div className="truncate text-xs text-muted">{m.email}</div>
                        </div>
                      </div>
                    </td>
                    <td className="px-2">
                      <select
                        aria-label={`Role for ${name}`}
                        className="select h-8 min-w-48 text-sm"
                        value={m.role_id ?? ""}
                        disabled={lock !== null || update.isPending}
                        title={lock ?? undefined}
                        onChange={(e) => change(saved, { role_id: e.target.value })}
                      >
                        {m.role_id ? null : (
                          <option value="" disabled>
                            Individual{m.based_on && roleName(m.based_on) ? `, based on ${roleName(m.based_on)}` : ""}
                          </option>
                        )}
                        {roles.map((r) => (
                          <option key={r.id} value={r.id} disabled={!assignableRole(actor, r)}>
                            {r.name}
                          </option>
                        ))}
                      </select>
                    </td>
                    <td className="px-2 text-xs text-muted">{when(m.joined_at)}</td>
                    <td className="space-x-2 px-3 text-right whitespace-nowrap">
                      <button
                        type="button"
                        className="btn btn-sm"
                        aria-expanded={expanded}
                        onClick={() => setOpen(expanded ? null : m.user_id)}
                      >
                        {expanded ? "Hide permissions" : "Permissions"}
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm btn-danger"
                        aria-label={`Remove ${name}`}
                        disabled={lock !== null || remove.isPending}
                        title={lock ?? undefined}
                        onClick={() => {
                          if (window.confirm(`Remove ${name} from this project? They lose access at once.`)) remove.mutate(m);
                        }}
                      >
                        Remove
                      </button>
                    </td>
                  </tr>
                  {expanded ? (
                    <tr className="bg-surface-2/50">
                      <td colSpan={4} className="px-4 py-3">
                        <p className="mb-3 text-xs text-muted">
                          {lock ??
                            (m.role_id
                              ? `Changing a box gives ${name} an individual set based on ${m.role_name}.`
                              : `${name} has an individual set of permissions.`)}
                        </p>
                        <fieldset aria-label={`Permissions of ${name}`} className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
                          {groups.map((g) => (
                            <div key={g.group}>
                              <div className="label mb-1.5">{g.group}</div>
                              {g.items.map((p) => {
                                const on = m.permissions.includes(p.id);
                                const outOfReach = !actor.permissions.includes(p.id);
                                return (
                                  <label key={p.id} className="flex items-center gap-2 py-0.5 text-sm">
                                    <input
                                      type="checkbox"
                                      checked={on}
                                      disabled={lock !== null || outOfReach || update.isPending}
                                      title={outOfReach ? "You do not have this permission yourself." : undefined}
                                      onChange={(e) =>
                                        change(saved, {
                                          permissions: toggle(m.permissions, p.id as PermissionId, e.target.checked),
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
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function RolesTab({ projectId, actor, catalog, roles }: { projectId: string; actor: Actor; catalog: CatalogEntry[]; roles: Role[] }) {
  const invalidate = useInvalidate(projectId);
  const drafts = useDrafts<{ name: string; permissions: string[] }>();
  const [name, setName] = useState("");
  const path = { project_id: projectId };
  const create = useMutation({
    mutationFn: async () => unwrap(await api.POST(ROLES, { params: { path }, body: { name, permissions: [] } })),
    onSuccess: () => setName(""),
    onSettled: invalidate,
  });
  const save = useMutation({
    mutationFn: async ({ role, name, permissions }: { role: Role; name: string; permissions: string[] }) =>
      unwrap(await api.PATCH(`${ROLES}/{role_id}`, { params: { path: { ...path, role_id: role.id } }, body: { name, permissions } })),
    onSettled: async (_data, _error, { role }) => {
      await invalidate();
      drafts.clear(role.id);
    },
  });
  const edit = (role: Role, name: string, permissions: string[]) => {
    drafts.set(role.id, { name, permissions });
    save.mutate({ role, name, permissions });
  };
  const shown = roles.map((r) => ({ ...r, ...drafts.get(r.id) }));
  const remove = useMutation({
    mutationFn: async (role: Role) =>
      unwrapEmpty(await api.DELETE(`${ROLES}/{role_id}`, { params: { path: { ...path, role_id: role.id } } })),
    onSettled: invalidate,
  });
  const groups = groupCatalog(catalog);
  const canManage = actor.permissions.includes("roles.manage");
  const error = create.error ?? save.error ?? remove.error;

  const onCreate = (e: FormEvent) => {
    e.preventDefault();
    create.mutate();
  };

  return (
    <div className="space-y-4">
      {canManage ? (
        <form onSubmit={onCreate} className="card flex flex-wrap items-end gap-3 p-4 text-sm">
          <label className="flex flex-col gap-1.5">
            <span className="label">Role name</span>
            <input required maxLength={64} value={name} onChange={(e) => setName(e.target.value)} className="input h-9 w-64" placeholder="e.g. Manager" />
          </label>
          <button type="submit" disabled={create.isPending} className="btn btn-primary h-9 px-4">
            <Icon name="plus" className="h-4 w-4" />
            Create role
          </button>
          <p className="basis-full text-xs text-muted">
            A new role starts without permissions; tick them below. You can only give a role permissions you have yourself.
          </p>
        </form>
      ) : null}

      {error ? (
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
          {message(error)}
        </p>
      ) : null}

      <div className="card overflow-x-auto">
        <table className="w-full text-left text-sm" aria-label="Roles">
          <thead className="table-head">
            <tr>
              <th className="px-4 py-2">Permission</th>
              {shown.map((r) => {
                const lock = roleLock(actor, r);
                return (
                  <th key={r.id} scope="col" className="min-w-32 px-2 py-2 text-center align-top">
                    <div className="font-medium text-text normal-case">{r.name}</div>
                    <div className="text-[11px] font-normal normal-case text-muted">
                      {r.builtin ? "built-in" : "custom"} · {r.holders} {r.holders === 1 ? "member" : "members"}
                    </div>
                    {r.builtin || lock ? null : (
                      <div className="mt-1 flex justify-center gap-1">
                        <button
                          type="button"
                          className="btn btn-sm"
                          aria-label={`Rename ${r.name}`}
                          onClick={() => {
                            const next = window.prompt("New role name:", r.name);
                            if (next && next.trim() && next.trim() !== r.name) edit(r, next.trim(), r.permissions);
                          }}
                        >
                          Rename
                        </button>
                        <button
                          type="button"
                          className="btn btn-sm btn-danger"
                          aria-label={`Delete ${r.name}`}
                          disabled={r.holders > 0 || remove.isPending}
                          title={r.holders > 0 ? "Give its members another role first." : undefined}
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
            {groups.map((g) => (
              <Fragment key={g.group}>
                <tr className="border-t border-border bg-surface-2/40">
                  <th colSpan={shown.length + 1} scope="colgroup" className="px-4 py-1.5 text-xs font-medium text-muted">
                    {g.group}
                  </th>
                </tr>
                {g.items.map((p) => (
                  <tr key={p.id} className="border-t border-border">
                    <th scope="row" className="px-4 py-2 font-normal">
                      {p.label}
                    </th>
                    {shown.map((r) => {
                      const lock = roleLock(actor, r);
                      const outOfReach = !actor.permissions.includes(p.id);
                      return (
                        <td key={r.id} className="px-2 text-center">
                          <input
                            type="checkbox"
                            aria-label={`${p.label} for ${r.name}`}
                            checked={r.permissions.includes(p.id)}
                            disabled={lock !== null || outOfReach}
                            title={lock ?? (outOfReach ? "You do not have this permission yourself." : undefined)}
                            onChange={(e) => edit(r, r.name, toggle(r.permissions, p.id as PermissionId, e.target.checked))}
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
    </div>
  );
}
