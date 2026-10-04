import { afterEach, describe, expect, it, vi } from "vitest";
import robots from "./robots";
import sitemap from "./sitemap";

afterEach(() => vi.unstubAllEnvs());

describe("robots", () => {
  it("keeps the private app and API out of the index", () => {
    const r = robots();
    const rules = Array.isArray(r.rules) ? r.rules : [r.rules];
    expect(rules).toHaveLength(1);
    expect(rules[0]?.userAgent).toBe("*");
    expect(rules[0]?.allow).toBe("/");
    expect(rules[0]?.disallow).toEqual(expect.arrayContaining(["/app", "/api"]));
  });

  it("points at the sitemap of the configured site", () => {
    vi.stubEnv("KARBIRD_SITE_URL", "https://karbird.example");
    expect(robots().sitemap).toBe("https://karbird.example/sitemap.xml");
  });

  it("falls back to localhost", () => {
    vi.stubEnv("KARBIRD_SITE_URL", undefined);
    expect(robots().sitemap).toBe("http://localhost:3000/sitemap.xml");
  });
});

describe("sitemap", () => {
  it("lists only public pages", () => {
    vi.stubEnv("KARBIRD_SITE_URL", "https://karbird.example");
    const entries = sitemap();
    const urls = entries.map((e) => e.url);
    expect(urls).toEqual([
      "https://karbird.example",
      "https://karbird.example/features",
      "https://karbird.example/integrations",
      "https://karbird.example/getting-started",
      "https://karbird.example/docs",
    ]);
    for (const u of urls) {
      expect(new URL(u).pathname.startsWith("/app")).toBe(false);
      expect(new URL(u).pathname.startsWith("/api")).toBe(false);
    }
  });

  it("gives the home page the top priority", () => {
    const [home, ...rest] = sitemap();
    expect(home?.priority).toBe(1);
    for (const e of rest) expect(e.priority).toBeLessThan(1);
  });
});
