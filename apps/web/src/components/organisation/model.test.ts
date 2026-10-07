import { describe, expect, it } from "vitest";
import { assignmentLabel, projectsForOrganization } from "./model";

describe("organization project filtering", () => {
  it("never shows assignments from another organization", () => {
    const projects = [
      { id: "a", organization_id: "one" },
      { id: "b", organization_id: "two" },
    ] as never;
    expect(projectsForOrganization(projects, "one").map((project) => project.id)).toEqual(["a"]);
  });
});

describe("organization assignment labels", () => {
  it("uses a role fallback and never invents a project name for a deleted assignment", () => {
    expect(assignmentLabel({ name: "Payments" } as never, undefined)).toBe("Payments · assigned");
    expect(assignmentLabel(undefined, "Operator")).toBe("Unknown project");
  });
});
