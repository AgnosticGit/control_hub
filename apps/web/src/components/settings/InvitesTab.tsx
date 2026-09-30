"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, useSyncExternalStore, type FormEvent } from "react";
import { Icon } from "@/components/ui/Icon";
import { api, apiSentence, unwrap, unwrapEmpty, type Schemas } from "@/lib/api";
import { EXPIRY_CHOICES, inviteLink, usesLabel } from "@/lib/invite";
import { assignableRole, defaultInviteRole, type Actor } from "./matrix";

type Role = Schemas["RoleDto"];

const INVITES = "/api/v1/projects/{project_id}/invites" as const;
const noSubscribe = () => () => {};

const message = (e: unknown) => apiSentence(e);

/** Form that creates an invite link and shows it once. */
export function InviteForm({ projectId, actor, roles, onCreated }: { projectId: string; actor: Actor; roles: Role[]; onCreated?: () => void }) {
  const origin = useSyncExternalStore(noSubscribe, () => window.location.origin, () => "");
  const [roleId, setRoleId] = useState<string>("");
  const [days, setDays] = useState<number>(7);
  const [single, setSingle] = useState(false);
  const [link, setLink] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const role = roles.find((r) => r.id === roleId) ?? defaultInviteRole(actor, roles);

  const create = useMutation({
    mutationFn: async () =>
      unwrap(
        await api.POST(INVITES, {
          params: { path: { project_id: projectId } },
          body: { role_id: role?.id ?? "", expires_in_days: days, max_uses: single ? 1 : null },
        }),
      ),
    onSuccess: (r) => {
      setLink(inviteLink(origin, r.secret));
      setCopied(false);
      onCreated?.();
    },
  });

  const submit = (e: FormEvent) => {
    e.preventDefault();
    create.mutate();
  };

  return (
    <div className="space-y-3 text-sm">
      <form onSubmit={submit} className="flex flex-wrap items-end gap-3">
        <label className="flex flex-col gap-1.5">
          <span className="label">Role for new members</span>
          <select className="select h-9 min-w-44" value={role?.id ?? ""} onChange={(e) => setRoleId(e.target.value)}>
            {roles.map((r) => (
              <option key={r.id} value={r.id} disabled={!assignableRole(actor, r)}>
                {r.name}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1.5">
          <span className="label">Link expires after</span>
          <select className="select h-9" value={days} onChange={(e) => setDays(Number(e.target.value))}>
            {EXPIRY_CHOICES.map((c) => (
              <option key={c.days} value={c.days}>
                {c.label}
              </option>
            ))}
          </select>
        </label>
        <label className="flex h-9 items-center gap-2">
          <input type="checkbox" checked={single} onChange={(e) => setSingle(e.target.checked)} />
          Single use
        </label>
        <button type="submit" disabled={!role || create.isPending} className="btn btn-primary h-9 px-4">
          <Icon name="link" className="h-4 w-4" />
          Create invite link
        </button>
      </form>
      {create.error ? (
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-error">
          {message(create.error)}
        </p>
      ) : null}
      {link ? (
        <div role="status" aria-label="New invite link" className="space-y-2 rounded-xl border border-warn/50 bg-warn/10 p-3">
          <p className="font-medium">Copy the link now; it is shown only once. Anyone with it can join as {role?.name}.</p>
          <div className="flex items-center gap-2">
            <code aria-label="Invite link" className="min-w-0 flex-1 truncate rounded-md border border-border bg-bg/80 px-2 py-1.5 font-mono text-xs">
              {link}
            </code>
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => {
                void navigator.clipboard.writeText(link).then(() => setCopied(true));
              }}
            >
              {copied ? "Copied" : "Copy link"}
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function InvitesTab({ projectId, actor, roles }: { projectId: string; actor: Actor; roles: Role[] }) {
  const qc = useQueryClient();
  const path = { project_id: projectId };
  const invites = useQuery({
    queryKey: ["invites", projectId],
    queryFn: async () => unwrap(await api.GET(INVITES, { params: { path } })),
  });
  const refresh = () => qc.invalidateQueries({ queryKey: ["invites", projectId] });
  const revoke = useMutation({
    mutationFn: async (id: string) => unwrapEmpty(await api.DELETE(`${INVITES}/{invite_id}`, { params: { path: { ...path, invite_id: id } } })),
    onSettled: refresh,
  });
  const error = invites.error ?? revoke.error;

  return (
    <div className="space-y-4">
      <section className="card space-y-3 p-4" aria-labelledby="new-invite">
        <h2 id="new-invite" className="text-base font-semibold">
          Invite people with a link
        </h2>
        <p className="text-sm text-muted">
          Whoever opens the link and signs in (or creates an account) joins this project with the chosen role. You can only
          invite with roles whose permissions you have yourself.
        </p>
        <InviteForm projectId={projectId} actor={actor} roles={roles} onCreated={() => void refresh()} />
      </section>

      {error ? (
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
          {message(error)}
        </p>
      ) : null}

      <div className="card overflow-hidden">
        <table className="w-full text-left text-sm" aria-label="Active invite links">
          <thead className="table-head">
            <tr>
              <th className="px-4 py-2">Link</th>
              <th className="px-2">Role</th>
              <th className="px-2">Created by</th>
              <th className="px-2">Expires</th>
              <th className="px-2">Uses</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {(invites.data ?? []).map((i) => (
              <tr key={i.id} className="border-t border-border">
                <td className="px-4 py-2.5 font-mono text-xs">{i.prefix}…</td>
                <td className="px-2">{i.role_name}</td>
                <td className="px-2 text-muted">{i.created_by_name}</td>
                <td className="px-2 text-xs text-muted">{new Date(i.expires_at).toLocaleString()}</td>
                <td className="px-2 text-xs text-muted">{usesLabel(i.uses, i.max_uses)}</td>
                <td className="px-3 text-right">
                  <button
                    type="button"
                    className="btn btn-sm btn-danger"
                    aria-label={`Revoke invite ${i.prefix}`}
                    disabled={revoke.isPending}
                    onClick={() => {
                      if (window.confirm("Revoke this invite link? Nobody can join with it any more.")) revoke.mutate(i.id);
                    }}
                  >
                    Revoke
                  </button>
                </td>
              </tr>
            ))}
            {invites.data && invites.data.length === 0 ? (
              <tr className="border-t border-border">
                <td colSpan={6} className="px-4 py-3 text-muted">
                  No active invite links.
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
    </div>
  );
}
