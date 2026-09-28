"use client";

import { createApiClient, unwrap, ApiRequestError, type Schemas } from "@controlhub/api-client";

export const api = createApiClient();
export { unwrap, ApiRequestError };
export type { Schemas };

export type SessionResponse = Schemas["SessionResponse"];
export type ProjectDto = Schemas["ProjectDto"];

export function scopedPath(projectId: string, environmentId: string) {
  return { project_id: projectId, environment_id: environmentId };
}

export function scopedUrl(projectId: string, environmentId: string, resource: string, query: Record<string, string>) {
  const qs = new URLSearchParams(query).toString();
  const base = `/api/v1/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(environmentId)}/${resource}`;
  return qs ? `${base}?${qs}` : base;
}

export function isUnauthenticated(e: unknown) {
  return e instanceof ApiRequestError && e.status === 401;
}
