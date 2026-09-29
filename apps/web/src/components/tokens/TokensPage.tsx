"use client";

import Link from "next/link";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { api, ApiRequestError, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { useSession } from "@/lib/session";

type TokenDto = Schemas["TokenDto"];
type TokenKind = Schemas["TokenKindDto"];
type Created = Schemas["CreatedTokenResponse"];

const BASE = "/api/v1/projects/{project_id}/environments/{environment_id}/tokens" as const;

function message(e: unknown) {
  return e instanceof ApiRequestError ? e.message : e instanceof Error ? e.message : String(e);
}

function when(iso: string | null | undefined) {
  return iso ? new Date(iso).toLocaleString() : "—";
}

export function TokensPage({ projectId, environmentId }: { projectId: string; environmentId: string }) {
  const session = useSession();
  const qc = useQueryClient();
  const path = scopedPath(projectId, environmentId);
  const [kind, setKind] = useState<TokenKind>("api");
  const [name, setName] = useState("");
  const [days, setDays] = useState("30");
  const [secret, setSecret] = useState<Created | null>(null);

  const project = session.data?.projects.find((p) => p.id === projectId);
  const env = project?.environments.find((e) => e.id === environmentId);
  const isAdmin = project?.role === "admin" || project?.role === "owner";

  const tokens = useQuery({
    queryKey: ["tokens", projectId, environmentId],
    enabled: isAdmin,
    queryFn: async () => unwrap(await api.GET(BASE, { params: { path } })),
  });
  const refresh = () => qc.invalidateQueries({ queryKey: ["tokens", projectId, environmentId] });

  const create = useMutation({
    mutationFn: async () =>
      unwrap(
        await api.POST(BASE, {
          params: { path },
          body: { kind, name, expires_in_days: days === "" ? null : Number(days) },
        }),
      ),
    onSuccess: (r) => {
      setSecret(r);
      setName("");
      void refresh();
    },
  });
  const revoke = useMutation({
    mutationFn: async (t: TokenDto) =>
      unwrap(await api.POST(`${BASE}/{token_id}/revoke`, { params: { path: { ...path, token_id: t.id } }, body: {} })),
    onSuccess: () => void refresh(),
  });
  const rotate = useMutation({
    mutationFn: async (t: TokenDto) =>
      unwrap(
        await api.POST(`${BASE}/{token_id}/rotate`, {
          params: { path: { ...path, token_id: t.id } },
          // Ingestion senders need time to pick up the new token; API tokens switch at once.
          body: { grace_seconds: t.kind === "ingest" ? 3600 : 0 },
        }),
      ),
    onSuccess: (r) => {
      setSecret(r);
      void refresh();
    },
  });

  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  if (!env) return <p className="p-8 text-sm">This environment does not exist or you do not have access to it.</p>;
  if (!isAdmin) return <p className="p-8 text-sm">Token management requires the admin role.</p>;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    create.mutate();
  };
  const error = create.error ?? revoke.error ?? rotate.error ?? tokens.error;

  return (
    <main className="mx-auto max-w-5xl p-8">
      <Link href={`/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}`} className="text-sm text-accent">
        ← Back to workspace
      </Link>
      <h1 className="mt-4 text-xl font-semibold">
        Tokens · {project?.name} / {env.name}
      </h1>
      <p className="mt-2 text-sm text-muted">
        Ingestion tokens let SDKs and collectors send telemetry to this environment. API tokens give read-only access
        to this environment (CLI <code>context</code>, MCP) and always expire. Secrets are shown once and stored only as
        hashes. Revocation of ingestion tokens takes effect within 30 seconds.
      </p>

      {secret ? (
        <section role="status" className="mt-6 rounded border border-warn/60 bg-warn/10 p-4 text-sm">
          <p className="font-medium">
            New {secret.token.kind === "api" ? "API" : "ingestion"} token “{secret.token.name}”. Copy it now; it will not
            be shown again.
          </p>
          <code aria-label="New token secret" className="mt-2 block break-all rounded bg-bg p-2 font-mono text-xs">
            {secret.secret}
          </code>
          <div className="mt-2 flex gap-2">
            <button
              type="button"
              className="rounded border border-border px-2 py-1 text-xs hover:border-accent"
              onClick={() => void navigator.clipboard.writeText(secret.secret)}
            >
              Copy
            </button>
            <button type="button" className="rounded border border-border px-2 py-1 text-xs" onClick={() => setSecret(null)}>
              Done
            </button>
          </div>
        </section>
      ) : null}

      <form onSubmit={onSubmit} className="mt-6 flex flex-wrap items-end gap-3 rounded border border-border bg-surface p-4 text-sm">
        <label className="flex flex-col gap-1">
          <span className="text-xs text-muted">Kind</span>
          <select
            value={kind}
            onChange={(e) => {
              const k = e.target.value === "ingest" ? "ingest" : "api";
              setKind(k);
              setDays(k === "api" ? "30" : "");
            }}
            className="rounded border border-border bg-bg px-2 py-1"
          >
            <option value="api">API (read-only)</option>
            <option value="ingest">Ingestion</option>
          </select>
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-xs text-muted">Name</span>
          <input
            required
            maxLength={64}
            pattern="[A-Za-z0-9 ._\-]+"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="w-56 rounded border border-border bg-bg px-2 py-1"
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-xs text-muted">Expires in days {kind === "ingest" ? "(empty = never)" : "(1–90)"}</span>
          <input
            type="number"
            min={1}
            max={kind === "api" ? 90 : 365}
            required={kind === "api"}
            value={days}
            onChange={(e) => setDays(e.target.value)}
            className="w-28 rounded border border-border bg-bg px-2 py-1"
          />
        </label>
        <button type="submit" disabled={create.isPending} className="rounded bg-accent px-3 py-1 font-medium text-bg disabled:opacity-50">
          Create token
        </button>
      </form>

      {error ? (
        <p role="alert" className="mt-4 text-sm text-error">
          {message(error)}
        </p>
      ) : null}

      <table className="mt-6 w-full text-left text-sm" aria-label="Tokens">
        <thead className="text-xs text-muted">
          <tr>
            <th className="py-2">Name</th>
            <th>Kind</th>
            <th>Prefix</th>
            <th>Created</th>
            <th>Expires</th>
            <th>Last used</th>
            <th>Status</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {(tokens.data ?? []).map((t) => (
            <tr key={t.id} className="border-t border-border">
              <td className="py-2">{t.name}</td>
              <td>{t.kind === "api" ? "API" : "Ingestion"}</td>
              <td className="font-mono text-xs">{t.prefix}…</td>
              <td>{when(t.created_at)}</td>
              <td>{t.expires_at ? when(t.expires_at) : "never"}</td>
              <td>{t.kind === "api" ? when(t.last_used_at) : "—"}</td>
              <td className={t.active ? "text-ok" : "text-muted"}>{t.active ? "active" : t.revoked_at ? "revoked" : "expired"}</td>
              <td className="space-x-2 text-right">
                {t.active ? (
                  <>
                    <button
                      type="button"
                      className="rounded border border-border px-2 py-0.5 text-xs hover:border-accent"
                      onClick={() => {
                        if (window.confirm(`Rotate “${t.name}”? ${t.kind === "ingest" ? "The old token keeps working for 1 hour." : "The old token stops working now."}`)) {
                          rotate.mutate(t);
                        }
                      }}
                    >
                      Rotate
                    </button>
                    <button
                      type="button"
                      className="rounded border border-error/60 px-2 py-0.5 text-xs text-error hover:bg-error/10"
                      onClick={() => {
                        if (window.confirm(`Revoke “${t.name}”? Senders using it will be rejected.`)) revoke.mutate(t);
                      }}
                    >
                      Revoke
                    </button>
                  </>
                ) : null}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </main>
  );
}
