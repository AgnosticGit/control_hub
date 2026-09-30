"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState, type FormEvent } from "react";
import { Icon } from "@/components/ui/Icon";
import { api, apiSentence, unwrap, unwrapEmpty, type ProjectDto } from "@/lib/api";
import { can } from "@/lib/permissions";
import { useSession } from "@/lib/session";
import { AccountShell } from "./AccountShell";

function envPath(p: ProjectDto, envId: string) {
  return `/app/${encodeURIComponent(p.id)}/${encodeURIComponent(envId)}`;
}

export function ProjectsPage() {
  const router = useRouter();
  const qc = useQueryClient();
  const session = useSession();
  const projects = session.data?.projects ?? [];
  const [name, setName] = useState("");
  const refresh = () => qc.invalidateQueries({ queryKey: ["session"] });

  const create = useMutation({
    mutationFn: async (projectName: string) => unwrap(await api.POST("/api/v1/projects", { body: { name: projectName } })),
    onSuccess: async (p) => {
      await refresh();
      const env = p.environments[0];
      router.push(env ? `${envPath(p, env.id)}/integration` : "/app");
    },
  });
  const rename = useMutation({
    mutationFn: async ({ id, name }: { id: string; name: string }) =>
      unwrapEmpty(await api.PATCH("/api/v1/projects/{project_id}", { params: { path: { project_id: id } }, body: { name } })),
    onSettled: refresh,
  });
  const leave = useMutation({
    mutationFn: async (id: string) => unwrapEmpty(await api.POST("/api/v1/projects/{project_id}/leave", { params: { path: { project_id: id } } })),
    onSettled: refresh,
  });
  const error = create.error ?? rename.error ?? leave.error;

  const submit = (e: FormEvent) => {
    e.preventDefault();
    create.mutate(name);
  };

  return (
    <AccountShell active="projects" title="Projects" description="Projects you belong to, with your role in each. Each project has its own members, roles and data.">
      <form onSubmit={submit} className="card flex flex-wrap items-end gap-3 p-5 text-sm" aria-label="New project">
        <label className="flex flex-col gap-1.5">
          <span className="label">Project name</span>
          <input value={name} onChange={(e) => setName(e.target.value)} required maxLength={100} placeholder="e.g. Shop backend" className="input h-9 w-72" />
        </label>
        <button type="submit" disabled={create.isPending || !name.trim()} className="btn btn-primary h-9 px-4">
          <Icon name="plus" className="h-4 w-4" />
          Create project
        </button>
        <p className="basis-full text-xs text-muted">You become its owner. It starts with one environment, “production”, and a guide to connect your services.</p>
      </form>

      {error ? (
        <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">
          {apiSentence(error)}
        </p>
      ) : null}

      <div className="card overflow-hidden">
        <table className="w-full text-left text-sm" aria-label="Your projects">
          <thead className="table-head">
            <tr>
              <th className="px-5 py-2">Project</th>
              <th className="px-2">Your role</th>
              <th className="px-2">Environments</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {projects.map((p) => (
              <tr key={p.id} className="border-t border-border">
                <td className="px-5 py-3">
                  <div className="flex items-center gap-2 font-medium">
                    <span className="flex h-6 w-6 items-center justify-center rounded-md bg-violet/20 text-xs font-bold text-violet">{p.name.slice(0, 1).toUpperCase()}</span>
                    {p.name}
                  </div>
                </td>
                <td className="px-2">
                  <span className="badge border-border-strong text-muted">{p.role_name}</span>
                </td>
                <td className="px-2">
                  <div className="flex flex-wrap gap-1.5">
                    {p.environments.map((e) => (
                      <Link key={e.id} href={envPath(p, e.id)} className="btn btn-sm">
                        {e.name}
                        {e.is_demo ? <span className="badge ml-1 border-warn/50 bg-warn/10 text-warn">DEMO</span> : null}
                      </Link>
                    ))}
                  </div>
                </td>
                <td className="space-x-2 px-5 text-right whitespace-nowrap">
                  {can(p, "project.manage") ? (
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
                  ) : null}
                  <button
                    type="button"
                    className="btn btn-sm btn-danger"
                    aria-label={`Leave ${p.name}`}
                    disabled={leave.isPending}
                    onClick={() => {
                      if (window.confirm(`Leave ${p.name}? You lose access until someone invites you again.`)) leave.mutate(p.id);
                    }}
                  >
                    Leave
                  </button>
                </td>
              </tr>
            ))}
            {projects.length === 0 ? (
              <tr className="border-t border-border">
                <td colSpan={4} className="px-5 py-3 text-muted">
                  You are not a member of any project yet. Create one above, or open an invite link.
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
    </AccountShell>
  );
}
