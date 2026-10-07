"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState, type FormEvent } from "react";
import { api, apiSentence, unwrap, unwrapEmpty } from "@/lib/api";
import { useSession } from "@/lib/session";
import { AccountShell } from "@/components/account/AccountShell";
import { projectsForOrganization } from "./model";

export function PeoplePage() {
  const session = useSession();
  const queryClient = useQueryClient();
  const organizations = session.data?.organizations ?? [];
  const [organizationId, setOrganizationId] = useState("");
  const [email, setEmail] = useState("");
  const [organizationRole, setOrganizationRole] = useState("member");
  const [projectId, setProjectId] = useState("");
  const [roleId, setRoleId] = useState("");

  useEffect(() => {
    if (!organizationId && organizations[0]) setOrganizationId(organizations[0].id);
  }, [organizationId, organizations]);

  const projects = useMemo(
    () => projectsForOrganization(session.data?.projects ?? [], organizationId),
    [organizationId, session.data?.projects],
  );
  useEffect(() => {
    if (!projects.some((project) => project.id === projectId)) {
      setProjectId(projects[0]?.id ?? "");
      setRoleId("");
    }
  }, [projectId, projects]);

  const members = useQuery({
    queryKey: ["organization-members", organizationId],
    enabled: Boolean(organizationId),
    queryFn: async () =>
      unwrap(await api.GET("/api/v1/organizations/{organization_id}/members", { params: { path: { organization_id: organizationId } } })),
  });
  const invites = useQuery({
    queryKey: ["organization-invites", organizationId],
    enabled: Boolean(organizationId),
    queryFn: async () =>
      unwrap(await api.GET("/api/v1/organizations/{organization_id}/invites", { params: { path: { organization_id: organizationId } } })),
  });
  const roles = useQuery({
    queryKey: ["roles", projectId],
    enabled: Boolean(projectId),
    queryFn: async () => unwrap(await api.GET("/api/v1/projects/{project_id}/roles", { params: { path: { project_id: projectId } } })),
  });
  useEffect(() => {
    if (!roleId && roles.data?.[0]) setRoleId(roles.data[0].id);
  }, [roleId, roles.data]);

  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["organization-members", organizationId] }),
      queryClient.invalidateQueries({ queryKey: ["organization-invites", organizationId] }),
    ]);
  };
  const invite = useMutation({
    mutationFn: async () =>
      unwrap(
        await api.POST("/api/v1/organizations/{organization_id}/invites", {
          params: { path: { organization_id: organizationId } },
          body: {
            email,
            organization_role: organizationRole,
            assignments: projectId && roleId ? [{ project_id: projectId, role_id: roleId }] : [],
            expires_in_days: 7,
          },
        }),
      ),
    onSuccess: async () => {
      setEmail("");
      await refresh();
    },
  });
  const revoke = useMutation({
    mutationFn: async (inviteId: string) =>
      unwrapEmpty(
        await api.DELETE("/api/v1/organizations/{organization_id}/invites/{invite_id}", {
          params: { path: { organization_id: organizationId, invite_id: inviteId } },
        }),
      ),
    onSuccess: refresh,
  });

  function submit(event: FormEvent) {
    event.preventDefault();
    invite.mutate();
  }

  return (
    <AccountShell active="people" title="People" description="Organization membership and project access are managed separately.">
      <label className="grid max-w-sm gap-1.5 text-sm font-medium">
        Organization
        <select className="input h-10" value={organizationId} onChange={(event) => setOrganizationId(event.target.value)}>
          {organizations.map((organization) => <option key={organization.id} value={organization.id}>{organization.name}</option>)}
        </select>
      </label>

      <form className="card grid gap-4 p-5 md:grid-cols-2" onSubmit={submit} aria-label="Invite a person">
        <label className="grid gap-1.5 text-sm font-medium md:col-span-2">
          Email
          <input className="input h-10" type="email" required value={email} onChange={(event) => setEmail(event.target.value)} />
        </label>
        <label className="grid gap-1.5 text-sm font-medium">
          Organization role
          <select className="input h-10" value={organizationRole} onChange={(event) => setOrganizationRole(event.target.value)}>
            <option value="member">Member</option>
            <option value="admin">Admin</option>
          </select>
        </label>
        <label className="grid gap-1.5 text-sm font-medium">
          Project
          <select className="input h-10" value={projectId} onChange={(event) => setProjectId(event.target.value)}>
            <option value="">No project yet</option>
            {projects.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}
          </select>
        </label>
        {projectId ? (
          <label className="grid gap-1.5 text-sm font-medium">
            Project role
            <select className="input h-10" value={roleId} onChange={(event) => setRoleId(event.target.value)} required>
              {(roles.data ?? []).map((role) => <option key={role.id} value={role.id}>{role.name}</option>)}
            </select>
          </label>
        ) : null}
        <div className="flex items-end">
          <button className="btn btn-primary h-10 px-4" disabled={invite.isPending || !email.trim() || !organizationId}>Send invitation</button>
        </div>
        {invite.error ? <p role="alert" className="text-sm text-error md:col-span-2">{apiSentence(invite.error)}</p> : null}
        {invite.data ? (
          <p role="status" className="text-sm text-muted md:col-span-2">
            Invitation {invite.data.invite.status}. Its one-time fallback token starts with {invite.data.invite.prefix}.
          </p>
        ) : null}
      </form>

      <section className="card overflow-hidden" aria-labelledby="members-heading">
        <h2 id="members-heading" className="border-b border-border px-5 py-3 font-semibold">Members</h2>
        <ul className="divide-y divide-border">
          {(members.data ?? []).map((member) => (
            <li key={member.user_id} className="flex flex-wrap items-center gap-3 px-5 py-3 text-sm">
              <div className="min-w-0 flex-1">
                <p className="font-medium">{member.display_name}</p>
                <p className="truncate text-muted">{member.email}</p>
              </div>
              <span className="badge border-border-strong">{member.role}</span>
              <span className="text-muted">{member.assignments.length} project{member.assignments.length === 1 ? "" : "s"}</span>
            </li>
          ))}
          {members.isPending ? <li className="px-5 py-3 text-sm text-muted">Loading…</li> : null}
        </ul>
      </section>

      <section className="card overflow-hidden" aria-labelledby="invites-heading">
        <h2 id="invites-heading" className="border-b border-border px-5 py-3 font-semibold">Invitations</h2>
        <ul className="divide-y divide-border">
          {(invites.data ?? []).map((item) => (
            <li key={item.id} className="flex flex-wrap items-center gap-3 px-5 py-3 text-sm">
              <span className="min-w-0 flex-1 truncate">{item.email}</span>
              <span className="badge border-border-strong">{item.status}</span>
              {!["accepted", "revoked", "expired"].includes(item.status) ? (
                <button className="btn btn-sm btn-danger" type="button" onClick={() => revoke.mutate(item.id)} disabled={revoke.isPending}>Revoke</button>
              ) : null}
            </li>
          ))}
          {!invites.isPending && invites.data?.length === 0 ? <li className="px-5 py-3 text-sm text-muted">No invitations.</li> : null}
        </ul>
      </section>
    </AccountShell>
  );
}

