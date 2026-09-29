import { afterEach, describe, expect, it, vi } from "vitest";

function directives(policy: string): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const part of policy.split(";")) {
    const [name, ...values] = part.trim().split(/\s+/);
    if (name) out.set(name, values);
  }
  return out;
}

async function load(nodeEnv: string) {
  vi.stubEnv("NODE_ENV", nodeEnv);
  vi.resetModules();
  return import("./csp");
}

afterEach(() => {
  vi.unstubAllEnvs();
  vi.resetModules();
});

describe("CSP", () => {
  it("strict policy (/app) requires the nonce and forbids inline script and JS eval in production", async () => {
    const { strictCsp } = await load("production");
    const d = directives(strictCsp("abc123=="));
    const script = d.get("script-src") ?? [];
    expect(script).toContain("'nonce-abc123=='");
    expect(script).toContain("'strict-dynamic'");
    expect(script).toContain("'wasm-unsafe-eval'");
    expect(script).not.toContain("'unsafe-inline'");
    expect(script).not.toContain("'unsafe-eval'");
  });

  it("baseline and strict policies lock down plugins, framing, forms and base URI", async () => {
    const { baselineCsp, strictCsp } = await load("production");
    for (const policy of [baselineCsp, strictCsp("n")]) {
      const d = directives(policy);
      expect(d.get("default-src")).toEqual(["'self'"]);
      expect(d.get("object-src")).toEqual(["'none'"]);
      expect(d.get("frame-ancestors")).toEqual(["'none'"]);
      expect(d.get("base-uri")).toEqual(["'self'"]);
      expect(d.get("form-action")).toEqual(["'self'"]);
      expect(d.get("connect-src")).toEqual(["'self'"]);
      expect(d.get("script-src")).not.toContain("'unsafe-eval'");
      expect(policy).not.toMatch(/https?:|\*/);
    }
  });

  it("baseline allows inline script only from self (static pages cannot carry a nonce)", async () => {
    const { baselineCsp } = await load("production");
    expect(directives(baselineCsp).get("script-src")).toEqual(["'self'", "'unsafe-inline'", "'wasm-unsafe-eval'"]);
  });

  it("allows JS eval only outside production (Next.js dev tooling)", async () => {
    const { strictCsp } = await load("development");
    expect(directives(strictCsp("n")).get("script-src")).toContain("'unsafe-eval'");
  });
});
