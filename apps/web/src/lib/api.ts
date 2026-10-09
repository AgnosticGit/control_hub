"use client";

import { createApiClient, unwrap, unwrapEmpty, ApiRequestError, type Schemas } from "@karbird/api-client";

export const api = createApiClient();
export { unwrap, unwrapEmpty, ApiRequestError };
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

export function errorMessage(e: unknown): string {
  return e instanceof ApiRequestError ? e.message : e instanceof Error ? e.message : String(e);
}

/** An API error message as a sentence for people: "conflict: you are…" → "You are….". */
export function apiSentence(e: unknown, fallback = "Cannot reach the karbird API."): string {
  if (!(e instanceof ApiRequestError)) return fallback;
  const text = e.message.replace(/^[a-z][a-z ]*: /, "").trim();
  if (!text) return fallback;
  const sentence = text.charAt(0).toUpperCase() + text.slice(1);
  return /[.!?]$/.test(sentence) ? sentence : `${sentence}.`;
}
