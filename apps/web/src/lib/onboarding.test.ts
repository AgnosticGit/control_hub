import { describe, expect, it } from "vitest";
import { nextStep, onboardingTarget, parseStep } from "./onboarding";

describe("steps", () => {
  it("parse from the URL, defaulting to the first step", () => {
    expect(parseStep("invite")).toBe("invite");
    expect(parseStep("done")).toBe("done");
    for (const bad of [null, undefined, "", "admin", "INVITE"]) expect(parseStep(bad)).toBe("connect");
  });

  it("advance in order and stop at the end", () => {
    expect(nextStep("connect")).toBe("invite");
    expect(nextStep("invite")).toBe("done");
    expect(nextStep("done")).toBe("done");
  });
});

describe("onboardingTarget", () => {
  const demo = { id: "demo", role: "viewer", permissions: [], environments: [{ id: "local-demo", is_demo: true }] };
  const mine = { id: "p-1", role: "owner", permissions: [], environments: [{ id: "e-demo", is_demo: true }, { id: "e-prod", is_demo: false }] };
  const empty = { id: "p-2", role: "owner", permissions: [], environments: [] };

  it("prefers a project you own and its real environment", () => {
    expect(onboardingTarget([demo, mine])).toMatchObject({ project: { id: "p-1" }, environment: { id: "e-prod" } });
  });

  it("honours a preferred project you belong to", () => {
    expect(onboardingTarget([mine, demo], "demo")).toMatchObject({ project: { id: "demo" }, environment: { id: "local-demo" } });
    expect(onboardingTarget([mine, demo], "not-mine")?.project.id).toBe("p-1");
  });

  it("keeps an owned demo project ahead of a real environment owned by someone else", () => {
    const ownedDemo = { id: "owned-demo", role: "owner", permissions: [], environments: [{ id: "e-owned", is_demo: true }] };
    const sharedProduction = { id: "shared-production", role: "viewer", permissions: [], environments: [{ id: "e-shared", is_demo: false }] };
    expect(onboardingTarget([sharedProduction, ownedDemo])).toMatchObject({
      project: { id: "owned-demo" },
      environment: { id: "e-owned" },
    });
  });

  it("skips projects without environments and handles having none", () => {
    expect(onboardingTarget([empty, demo])?.project.id).toBe("demo");
    expect(onboardingTarget([empty])).toBeNull();
    expect(onboardingTarget([])).toBeNull();
  });
});
