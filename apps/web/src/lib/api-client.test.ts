// Tests for `@karbird/api-client` (the package has no test runner of its own).
import { ApiRequestError, createApiClient, unwrap, unwrapEmpty } from "@karbird/api-client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const BASE = "https://karbird.test";

let seen: Request[];
let reply: () => Response;

beforeEach(() => {
  seen = [];
  reply = () => new Response(JSON.stringify({ ok: true }), { status: 200, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      seen.push(input instanceof Request ? input : new Request(input, init));
      return reply();
    }),
  );
  vi.stubGlobal("document", { cookie: "theme=dark; karbird_csrf=tok%3D123; other=x" });
});

afterEach(() => vi.unstubAllGlobals());

function last(): Request {
  const r = seen.at(-1);
  if (!r) throw new Error("no request was sent");
  return r;
}

describe("createApiClient", () => {
  it("adds the CSRF double-submit header to mutations", async () => {
    const api = createApiClient(BASE);
    await api.POST("/api/v1/session/logout");
    expect(last().method).toBe("POST");
    expect(last().headers.get("x-csrf-token")).toBe("tok=123");
  });

  it("adds it to mutations with a body and path parameters", async () => {
    const api = createApiClient(BASE);
    await api.POST("/api/v1/projects/{project_id}/environments/{environment_id}/tokens/{token_id}/revoke", {
      params: { path: { project_id: "p/1", environment_id: "e1", token_id: "t1" } },
      body: { reason: "leaked" },
    });
    expect(new URL(last().url).pathname).toBe("/api/v1/projects/p%2F1/environments/e1/tokens/t1/revoke");
    expect(last().headers.get("x-csrf-token")).toBe("tok=123");
    expect(await last().json()).toEqual({ reason: "leaked" });
  });

  it("does not send the CSRF token on reads", async () => {
    const api = createApiClient(BASE);
    await api.GET("/api/v1/session");
    expect(last().method).toBe("GET");
    expect(last().headers.has("x-csrf-token")).toBe(false);
  });

  it("sends no CSRF header when the cookie is missing", async () => {
    vi.stubGlobal("document", { cookie: "karbird_csrf_other=x" });
    const api = createApiClient(BASE);
    await api.POST("/api/v1/session/logout");
    expect(last().headers.has("x-csrf-token")).toBe(false);
  });

  it("works outside the browser (no document)", async () => {
    vi.stubGlobal("document", undefined);
    const api = createApiClient(BASE);
    await api.POST("/api/v1/session/logout");
    expect(last().headers.has("x-csrf-token")).toBe(false);
  });

  it("uses same-origin credentials (the HttpOnly session cookie, never a bearer token)", async () => {
    const api = createApiClient(BASE);
    await api.GET("/api/v1/session");
    expect(last().credentials).toBe("same-origin");
    expect(last().headers.has("authorization")).toBe(false);
  });
});

describe("unwrap", () => {
  it("returns data of a successful response", () => {
    expect(unwrap({ data: { a: 1 }, response: new Response(null, { status: 200 }) })).toEqual({ a: 1 });
  });

  it("throws a typed error with the server's code and message", () => {
    const run = () => unwrap({ error: { code: "forbidden", message: "not allowed" }, response: new Response(null, { status: 403 }) });
    expect(run).toThrow(ApiRequestError);
    try {
      run();
    } catch (e) {
      expect(e).toMatchObject({ name: "ApiRequestError", status: 403, code: "forbidden", message: "not allowed" });
    }
  });

  it("falls back to a generic error for untyped bodies", () => {
    try {
      unwrap({ error: "<html>bad gateway</html>", response: new Response(null, { status: 502 }) });
      expect.unreachable();
    } catch (e) {
      expect(e).toMatchObject({ status: 502, code: "http_error", message: "HTTP 502" });
    }
  });

  it("treats data on a non-OK response as an error", () => {
    expect(() => unwrap({ data: { a: 1 }, response: new Response(null, { status: 500 }) })).toThrow(ApiRequestError);
  });

  it("unwrapEmpty accepts a 204 without a body and still throws on errors", async () => {
    reply = () => new Response(null, { status: 204 });
    const api = createApiClient(BASE);
    const path = { project_id: "demo", role_id: "r1" };
    await expect(api.DELETE("/api/v1/projects/{project_id}/roles/{role_id}", { params: { path } }).then(unwrapEmpty)).resolves.toBeUndefined();
    expect(last().method).toBe("DELETE");
    expect(last().headers.get("x-csrf-token")).toBe("tok=123");
    reply = () =>
      new Response(JSON.stringify({ code: "conflict", message: "the role is still assigned to members" }), {
        status: 409,
        headers: { "content-type": "application/json" },
      });
    await expect(api.DELETE("/api/v1/projects/{project_id}/roles/{role_id}", { params: { path } }).then(unwrapEmpty)).rejects.toMatchObject({
      status: 409,
      code: "conflict",
    });
  });

  it("round-trips a real 401 from the client", async () => {
    reply = () =>
      new Response(JSON.stringify({ code: "unauthenticated", message: "no session" }), {
        status: 401,
        headers: { "content-type": "application/json" },
      });
    const api = createApiClient(BASE);
    await expect(api.GET("/api/v1/session").then(unwrap)).rejects.toMatchObject({ status: 401, code: "unauthenticated" });
  });
});
