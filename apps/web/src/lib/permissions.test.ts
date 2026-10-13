import { describe, expect, it } from "vitest";
import { PERMISSIONS, can, canCancelPlan, canDecidePlan, canStartAction, roleLabel } from "./permissions";

const project = (permissions: string[], role = "custom", role_name = "Manager") => ({ permissions, role, role_name });

describe("can", () => {
  it("is true only for permissions the project grants", () => {
    const p = project(["telemetry.view", "tokens.view"]);
    expect(can(p, "telemetry.view")).toBe(true);
    expect(can(p, "tokens.view")).toBe(true);
    expect(can(p, "tokens.manage")).toBe(false);
  });

  it("is false without a project (not loaded or no access)", () => {
    expect(can(undefined, "telemetry.view")).toBe(false);
    expect(can(null, "telemetry.view")).toBe(false);
  });

  it("does not infer permissions from the role name", () => {
    expect(can(project([], "owner", "Owner"), "tokens.manage")).toBe(false);
    expect(can(project([], "admin", "Admin"), "members.manage")).toBe(false);
  });

  it("knows every permission the API catalog has", () => {
    expect(PERMISSIONS).toEqual([
      "telemetry.view",
      "incidents.manage",
      "tokens.view",
      "tokens.manage",
      "actions.view",
      "actions.propose",
      "actions.approve",
      "members.view",
      "members.manage",
      "roles.manage",
      "project.manage",
    ]);
  });
});

describe("plan decisions", () => {
  const approver = project(["actions.view", "actions.approve"]);
  const proposer = project(["actions.view", "actions.propose"]);

  const viewer = project(["actions.view"]);

  it("decisions depend only on the approve permission", () => {
    expect(canDecidePlan(approver)).toBe(true);
    expect(canDecidePlan(proposer)).toBe(false);
    expect(canDecidePlan(viewer)).toBe(false);
    expect(canDecidePlan(undefined)).toBe(false);
  });

  it("approvers and proposers can start actions, viewers cannot", () => {
    expect(canStartAction(approver)).toBe(true);
    expect(canStartAction(proposer)).toBe(true);
    expect(canStartAction(viewer)).toBe(false);
  });

  it("your own plan can be cancelled by whoever could start it, someone else's only by approvers", () => {
    expect(canCancelPlan(proposer, true)).toBe(true);
    expect(canCancelPlan(proposer, false)).toBe(false);
    expect(canCancelPlan(approver, false)).toBe(true);
    expect(canCancelPlan(approver, true)).toBe(true);
    expect(canCancelPlan(viewer, true)).toBe(false);
  });
});

describe("roleLabel", () => {
  it("shows the role name and marks individual sets", () => {
    expect(roleLabel(project([], "owner", "Owner"))).toBe("Owner");
    expect(roleLabel(project([], "custom", "Manager"))).toBe("Manager");
    expect(roleLabel(project([], "individual", "Individual"))).toBe("Individual");
    expect(roleLabel(undefined)).toBe("");
  });
});
