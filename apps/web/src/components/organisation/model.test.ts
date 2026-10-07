import { describe, expect, it } from "vitest";
import { projectsForOrganization } from "./model";

describe("organization project filtering", () => {
  it("never shows assignments from another organization", () => {
    const projects = [
      { id: "a", organization_id: "one" },
      { id: "b", organization_id: "two" },
    ] as never;
    expect(projectsForOrganization(projects, "one").map((project) => project.id)).toEqual(["a"]);
  });
});

