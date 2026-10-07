"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { useState, type FormEvent } from "react";
import { api, apiSentence, unwrap } from "@/lib/api";

export const DEFAULT_ENVIRONMENTS = ["Development", "Staging", "Production"];

export function ProjectWizard() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const [organization, setOrganization] = useState("");
  const [project, setProject] = useState("");
  const [environments, setEnvironments] = useState(DEFAULT_ENVIRONMENTS);
  const create = useMutation({
    mutationFn: async () =>
      unwrap(
        await api.POST("/api/v1/onboarding/first-project", {
          body: { organization_name: organization, project_name: project, environments },
        }),
      ),
    onSuccess: async (created) => {
      await queryClient.invalidateQueries({ queryKey: ["session"] });
      const environment = created.environments[0];
      router.replace(environment ? `/app/${created.id}/${environment.id}/integration` : "/app");
    },
  });

  function submit(event: FormEvent) {
    event.preventDefault();
    create.mutate();
  }

  return (
    <form onSubmit={submit} className="mx-auto max-w-xl space-y-6" aria-label="Create your workspace">
      <div>
        <p className="text-sm font-semibold text-accent">Set up your workspace</p>
        <h1 className="mt-2 text-3xl font-bold tracking-tight">Your first project</h1>
        <p className="mt-2 text-muted">An organization contains people and projects. Environments keep development, staging and production data separate.</p>
      </div>
      <div className="card grid gap-4 p-5">
        <label className="grid gap-1.5 text-sm font-medium">
          Organization name
          <input className="input h-10" value={organization} onChange={(event) => setOrganization(event.target.value)} required maxLength={100} placeholder="Acme" />
        </label>
        <label className="grid gap-1.5 text-sm font-medium">
          First project
          <input className="input h-10" value={project} onChange={(event) => setProject(event.target.value)} required maxLength={100} placeholder="Storefront" />
        </label>
        <fieldset className="grid gap-2">
          <legend className="text-sm font-medium">Environments</legend>
          {environments.map((name, index) => (
            <div key={`${index}-${name}`} className="flex gap-2">
              <input
                className="input h-9 flex-1"
                value={name}
                aria-label={`Environment ${index + 1}`}
                maxLength={100}
                onChange={(event) => setEnvironments((items) => items.map((item, i) => (i === index ? event.target.value : item)))}
              />
              <button type="button" className="btn btn-sm" disabled={environments.length === 1} onClick={() => setEnvironments((items) => items.filter((_, i) => i !== index))}>
                Remove
              </button>
            </div>
          ))}
          <button type="button" className="btn btn-sm w-fit" onClick={() => setEnvironments((items) => [...items, ""])}>Add environment</button>
        </fieldset>
      </div>
      {create.error ? <p role="alert" className="rounded-md border border-error/40 bg-error/10 px-3 py-2 text-sm text-error">{apiSentence(create.error)}</p> : null}
      <button type="submit" className="btn btn-primary h-10 px-5" disabled={create.isPending || !organization.trim() || !project.trim()}>
        {create.isPending ? "Creating…" : "Create workspace"}
      </button>
    </form>
  );
}
