import { describe, expect, it } from "vitest";
import { keptWorkspace, parseWorkspacePath, sameRoute } from "./workspace-route";

const demo = { projectId: "demo", environmentId: "demo-env" };

describe("parseWorkspacePath", () => {
  it("recognises the overview and the other pages of an environment", () => {
    expect(parseWorkspacePath("/app/demo/demo-env")).toEqual({ route: demo, overview: true });
    expect(parseWorkspacePath("/app/demo/demo-env/")).toEqual({ route: demo, overview: true });
    expect(parseWorkspacePath("/app/demo/demo-env/tokens")).toEqual({ route: demo, overview: false });
  });

  it("decodes ids and rejects malformed ones", () => {
    expect(parseWorkspacePath("/app/a%20b/e%2F1")?.route).toEqual({ projectId: "a b", environmentId: "e/1" });
    expect(parseWorkspacePath("/app/%E0%A4%A/env")).toBeNull();
  });

  it("is null for pages outside a project", () => {
    for (const p of ["/", "/app", "/app/account", "/app/account/profile", "/app/admin", "/app/admin/projects", "/app/login", "/docs/a/b"]) {
      expect(parseWorkspacePath(p)).toBeNull();
    }
  });
});

describe("keptWorkspace", () => {
  it("starts keeping a workspace only once its overview is opened", () => {
    expect(keptWorkspace(null, "/app/demo/demo-env/tokens")).toBeNull();
    expect(keptWorkspace(null, "/app/demo/demo-env")).toEqual(demo);
  });

  it("keeps it on the environment's other pages and outside projects, as the same object", () => {
    for (const p of ["/app/demo/demo-env/tokens", "/app/demo/demo-env/settings", "/app/account", "/app/admin", "/app", "/app/demo/demo-env"]) {
      expect(keptWorkspace(demo, p)).toBe(demo);
    }
  });

  it("drops it on a page of another environment and switches on another overview", () => {
    expect(keptWorkspace(demo, "/app/other/prod/tokens")).toBeNull();
    expect(keptWorkspace(demo, "/app/demo/staging/settings")).toBeNull();
    expect(keptWorkspace(demo, "/app/other/prod")).toEqual({ projectId: "other", environmentId: "prod" });
  });
});

describe("sameRoute", () => {
  it("compares by value", () => {
    expect(sameRoute(demo, { ...demo })).toBe(true);
    expect(sameRoute(demo, null)).toBe(false);
    expect(sameRoute(null, null)).toBe(true);
  });
});
