import { describe, expect, it } from "vitest";
import { workspaceSuffix, workspaceTarget } from "./scope-switcher";

describe("workspace switcher routes", () => {
  it("preserves a valid subpage while changing scope", () => {
    expect(workspaceSuffix("/app/old/dev/tokens")).toBe("/tokens");
    expect(workspaceTarget("new project", "staging", "/app/old/dev/tokens")).toBe("/app/new%20project/staging/tokens");
  });

  it("uses the workspace overview for an overview route", () => {
    expect(workspaceTarget("new", "prod", "/app/old/dev")).toBe("/app/new/prod");
  });
});

