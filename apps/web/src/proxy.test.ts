import { NextRequest } from "next/server";
import { describe, expect, it } from "vitest";
import { config, isPublicAppPath, proxy } from "./proxy";

const ORIGIN = "https://karbird.test";

function request(path: string, session: string | null = "s3cr3t-session") {
  const headers = new Headers();
  if (session !== null) headers.set("cookie", `karbird_session=${session}`);
  return new NextRequest(`${ORIGIN}${path}`, { headers });
}

function scriptSrc(csp: string | null): string[] {
  const part = (csp ?? "").split(";").find((p) => p.trim().startsWith("script-src"));
  return part ? part.trim().split(/\s+/).slice(1) : [];
}

describe("proxy (/app)", () => {
  it("only runs for the private app", () => {
    expect(config.matcher).toEqual(["/app", "/app/:path*"]);
  });

  it("redirects to login without a session cookie and remembers the page", () => {
    const res = proxy(request("/app/p1/e1/actions", null));
    expect(res.status).toBe(307);
    const location = new URL(res.headers.get("location") ?? "");
    expect(location.pathname).toBe("/app/login");
    expect(location.searchParams.get("next")).toBe("/app/p1/e1/actions");
    expect(location.origin).toBe(ORIGIN);
  });

  it("does not forward the original query string into `next`", () => {
    const res = proxy(request("/app/p1?token=abc", null));
    const location = new URL(res.headers.get("location") ?? "");
    expect(location.searchParams.get("next")).toBe("/app/p1");
    expect(location.search).not.toContain("token");
  });

  it("lets the login page through without a session", () => {
    for (const path of ["/app/login", "/app/login/"]) {
      const res = proxy(request(path, null));
      expect(res.headers.get("location")).toBeNull();
      expect(res.headers.get("content-security-policy")).toContain("'nonce-");
    }
  });

  it("lets the sign-up page through without a session, with the strict CSP", () => {
    for (const path of ["/app/register", "/app/register/"]) {
      const res = proxy(request(path, null));
      expect(res.headers.get("location")).toBeNull();
      expect(res.headers.get("content-security-policy")).toContain("'nonce-");
    }
  });

  it("lets the invite page through without a session, so a visitor sees the invite before signing in", () => {
    const res = proxy(request("/app/invite", null));
    expect(res.headers.get("location")).toBeNull();
    expect(res.headers.get("content-security-policy")).toContain("'nonce-");
    expect(res.headers.get("cache-control")).toBe("no-store");
  });

  it("does not treat look-alike paths as public pages", () => {
    for (const path of ["/app/loginx", "/app/registerx", "/app/p1/register", "/app/register-admin", "/app/invites", "/app/p1/invite"]) {
      expect(isPublicAppPath(path)).toBe(false);
      expect(proxy(request(path, null)).status).toBe(307);
    }
  });

  it("sets a strict nonce CSP on the response and forwards it to Next.js on the request", () => {
    const res = proxy(request("/app"));
    const csp = res.headers.get("content-security-policy");
    const script = scriptSrc(csp);
    const nonce = script.find((s) => s.startsWith("'nonce-"));
    expect(nonce).toMatch(/^'nonce-[A-Za-z0-9+/]{22}=='$/);
    expect(script).toContain("'strict-dynamic'");
    expect(script).not.toContain("'unsafe-inline'");
    expect(csp).toContain("frame-ancestors 'none'");
    expect(res.headers.get("x-middleware-request-content-security-policy")).toBe(csp);
  });

  it("uses a fresh nonce on every request", () => {
    const nonces = new Set<string | undefined>();
    for (let i = 0; i < 20; i++) {
      nonces.add(scriptSrc(proxy(request("/app")).headers.get("content-security-policy")).find((s) => s.startsWith("'nonce-")));
    }
    expect(nonces.size).toBe(20);
  });

  it("never echoes the session cookie into the CSP", () => {
    const res = proxy(request("/app", "very-secret-value"));
    expect(res.headers.get("content-security-policy")).not.toContain("very-secret-value");
  });
});
