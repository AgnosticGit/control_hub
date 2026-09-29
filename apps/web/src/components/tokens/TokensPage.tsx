"use client";

import Link from "next/link";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent, type ReactNode } from "react";
import { AppShell } from "@/components/shell/AppShell";
import { Icon } from "@/components/ui/Icon";
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

function Centered({ children }: { children: ReactNode }) {
  return (
    <main className="flex min-h-screen items-center justify-center p-8">
      <div className="card max-w-md p-6 text-sm">
        {children}
        <Link href="/app" className="btn mt-4">
          <Icon name="arrowLeft" className="h-3.5 w-3.5" />
          Back to environments
        </Link>
      </div>
    </main>
  );
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
  if (!env) {
    return (
      <Centered>
        <p>This environment does not exist or you do not have access to it.</p>
      </Centered>
    );
  }
  if (!isAdmin) {
    return (
      <Centered>
        <p>Token management requires the admin role.</p>
      </Centered>
    );
  }

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    create.mutate();
  };
  const error = create.error ?? revoke.error ?? rotate.error ?? tokens.error;

  return (
    <AppShell
      projectId={projectId}
      environmentId={environmentId}
      active="tokens"
      top={
        <>
          <span className="card-title-icon">
            <Icon name="key" className="h-3.5 w-3.5" />
          </span>
          <span className="text-sm text-muted">
            {project?.name} / {env.name}
          </span>
        </>
      }
    >
      <main className="h-full overflow-auto">
        <div className="mx-auto max-w-6xl space-y-5 p-6">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">Tokens</h1>
            <p className="mt-1 max-w-3xl text-sm text-muted">
              Ingestion tokens let SDKs and collectors send telemetry to this environment. API tokens give read-only access
              to this environment (CLI <code className="font-mono">context</code>, MCP) and always expire. Secrets are shown
              once and stored only as hashes. Revocation of ingestion tokens takes effect within 30 seconds.
            </p>
          </div>

          {secret ? (
            <section role="status" className="rounded-xl border border-warn/50 bg-warn/10 p-4 text-sm">
              <p className="font-medium">
                New {secret.token.kind === "api" ? "API" : "ingestion"} token “{secret.token.name}”. Copy it now; it will not
                be shown again.
              </p>
              <code aria-label="New token secret" className="mt-2 block rounded-lg border border-border bg-bg/80 p-3 font-mono text-xs break-all">
                {secret.secret}
              </code>
              <div className="mt-3 flex gap-2">
                <button type="button" className="btn btn-sm" onClick={() => void navigator.clipboard.writeText(secret.secret)}>
                  Copy
                </button>
                <button type="button" className="btn btn-sm" onClick={() => setSecret(null)}>
                  Done
                </button>
              </div>
            </section>
          ) : null}

          <form onSubmit={onSubmit} className="card flex flex-wrap items-end gap-3 p-5 text-sm">
            <label className="flex flex-col gap-1.5">
              <span className="label">Kind</span>
              <select
                value={kind}
                onChange={(e) => {
                  const k = e.target.value === "ingest" ? "ingest" : "api";
                  setKind(k);
                  setDays(k === "api" ? "30" : "");
                }}
                className="select h-9 min-w-44 text-sm"
              >
                <option value="api">API (read-only)</option>
                <option value="ingest">Ingestion</option>
              </select>
            </label>
            <label className="flex flex-col gap-1.5">
              <span className="label">Name</span>
              <input
                required
                maxLength={64}
                pattern="[A-Za-z0-9 ._\-]+"
                value={name}
                onChange={(e) => setName(e.target.value)}
                className="input h-9 w-56"
              />
            </label>
            <label className="flex flex-col gap-1.5">
              <span className="label">Expires in days {kind === "ingest" ? "(empty = never)" : "(1–90)"}</span>
              <input
                type="number"
                min={1}
                max={kind === "api" ? 90 : 365}
                required={kind === "api"}
                value={days}
                onChange={(e) => setDays(e.target.value)}
                className="input h-9 w-28"
              />
            </label>
            <button type="submit" disabled={create.isPending} className="btn btn-primary h-9 px-4">
              <Icon name="plus" className="h-4 w-4" />
              Create token
            </button>
          </form>

          {error ? (
            <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
              {message(error)}
            </p>
          ) : null}

          <div className="card overflow-hidden">
            <table className="w-full text-left text-sm" aria-label="Tokens">
              <thead className="table-head">
                <tr>
                  <th className="px-4 py-2">Name</th>
                  <th className="px-2">Kind</th>
                  <th className="px-2">Prefix</th>
                  <th className="px-2">Created</th>
                  <th className="px-2">Expires</th>
                  <th className="px-2">Last used</th>
                  <th className="px-2">Status</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {(tokens.data ?? []).map((t) => (
                  <tr key={t.id} className="border-t border-border transition-colors hover:bg-surface-2">
                    <td className="px-4 py-2.5 font-medium">{t.name}</td>
                    <td className="px-2">{t.kind === "api" ? "API" : "Ingestion"}</td>
                    <td className="px-2 font-mono text-xs text-muted">{t.prefix}…</td>
                    <td className="px-2 text-xs text-muted">{when(t.created_at)}</td>
                    <td className="px-2 text-xs text-muted">{t.expires_at ? when(t.expires_at) : "never"}</td>
                    <td className="px-2 text-xs text-muted">{t.kind === "api" ? when(t.last_used_at) : "—"}</td>
                    <td className="px-2">
                      <span className={`badge ${t.active ? "border-ok/40 bg-ok/10 text-ok" : "border-border-strong text-muted"}`}>
                        {t.active ? "active" : t.revoked_at ? "revoked" : "expired"}
                      </span>
                    </td>
                    <td className="space-x-2 px-3 text-right whitespace-nowrap">
                      {t.active ? (
                        <>
                          <button
                            type="button"
                            className="btn btn-sm"
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
                            className="btn btn-sm btn-danger"
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
            {(tokens.data ?? []).length === 0 && !tokens.isPending ? (
              <p className="border-t border-border px-4 py-6 text-center text-sm text-muted">No tokens yet.</p>
            ) : null}
          </div>
        </div>
      </main>
    </AppShell>
  );
}
