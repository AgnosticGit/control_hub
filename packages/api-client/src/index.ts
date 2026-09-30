import createClient, { type Middleware } from "openapi-fetch";
import type { components, paths } from "./generated/schema";

export type { components, paths };
export type Schemas = components["schemas"];

const CSRF_COOKIE = "controlhub_csrf";
const CSRF_HEADER = "x-csrf-token";

function readCookie(name: string): string | null {
  if (typeof document === "undefined") return null;
  for (const part of document.cookie.split(";")) {
    const [k, ...v] = part.trim().split("=");
    if (k === name) return decodeURIComponent(v.join("="));
  }
  return null;
}

const csrf: Middleware = {
  onRequest({ request }) {
    if (request.method !== "GET" && request.method !== "HEAD") {
      const token = readCookie(CSRF_COOKIE);
      if (token) request.headers.set(CSRF_HEADER, token);
    }
    return request;
  },
};

/**
 * Browser client for the ControlHub REST API. Uses same-origin requests with
 * the HttpOnly session cookie; the CSRF double-submit header is added to
 * every mutation automatically.
 */
export function createApiClient(baseUrl = "") {
  const client = createClient<paths>({ baseUrl, credentials: "same-origin" });
  client.use(csrf);
  return client;
}

export type ApiClient = ReturnType<typeof createApiClient>;

export class ApiRequestError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiRequestError";
  }
}

function requestError(result: { error?: unknown; response: Response }): ApiRequestError {
  const err = result.error as { code?: unknown; message?: unknown } | undefined;
  return new ApiRequestError(
    result.response.status,
    typeof err?.code === "string" ? err.code : "http_error",
    typeof err?.message === "string" ? err.message : `HTTP ${result.response.status}`,
  );
}

/** Unwraps an openapi-fetch result, throwing a typed error on failure. */
export function unwrap<T>(result: { data?: T; error?: unknown; response: Response }): T {
  if (result.data !== undefined && result.response.ok) return result.data;
  throw requestError(result);
}

/** For responses without a body (204): succeeds on any 2xx, throws a typed error otherwise. */
export function unwrapEmpty(result: { error?: unknown; response: Response }): void {
  if (!result.response.ok) throw requestError(result);
}
