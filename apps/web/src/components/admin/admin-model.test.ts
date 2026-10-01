import { describe, expect, it } from "vitest";
import { assignableRole, memberLock, normalise, roleLock, toggle, type Actor } from "@/components/settings/matrix";
import {
  accountLock,
  adminTabs,
  auditActionLabel,
  canAdmin,
  projectDeleteLock,
  SYSTEM_MODEL,
  SYSTEM_PERMISSIONS,
  systemActor,
} from "./admin-model";

const ALL = [...SYSTEM_PERMISSIONS];
const AUDITOR = ["system.projects.view", "system.users.view", "system.team.view", "system.audit.view"];
const superAdmin: Actor = { userId: "sa", permissions: ALL, isOwner: true };
const admin: Actor = { userId: "ad", permissions: ALL, isOwner: false };
const auditor: Actor = { userId: "au", permissions: AUDITOR, isOwner: false };

describe("system permissions in the shared matrix", () => {
  it("normalise adds implied ids, drops project ids and keeps catalog order", () => {
    expect(normalise(["system.roles.manage", "telemetry.view", "system.users.manage"], SYSTEM_MODEL)).toEqual([
      "system.users.view",
      "system.users.manage",
      "system.team.view",
      "system.roles.manage",
    ]);
  });

  it("unticking a view unticks what needs it", () => {
    expect(toggle(["system.team.view", "system.team.manage", "system.roles.manage"], "system.team.view", false, SYSTEM_MODEL)).toEqual([]);
    expect(toggle([], "system.projects.manage", true, SYSTEM_MODEL)).toEqual(["system.projects.view", "system.projects.manage"]);
  });

  it("only a Super Admin changes or grants the Super Admin", () => {
    const sa = { user_id: "sa2", permissions: ALL, is_owner: true };
    expect(memberLock(admin, sa, SYSTEM_MODEL)).toMatch(/Super Admin/);
    expect(memberLock(superAdmin, sa, SYSTEM_MODEL)).toBeNull();
    expect(memberLock(superAdmin, { ...sa, user_id: "sa" }, SYSTEM_MODEL)).toMatch(/your own/);
    const saRole = { permissions: ALL, builtin: "super_admin" };
    expect(assignableRole(admin, saRole, SYSTEM_MODEL)).toBe(false);
    expect(assignableRole(superAdmin, saRole, SYSTEM_MODEL)).toBe(true);
  });

  it("an Auditor changes nothing and grants nothing beyond its own permissions", () => {
    expect(memberLock(auditor, { user_id: "x", permissions: [], is_owner: false }, SYSTEM_MODEL)).toMatch(/Grant and change/);
    expect(roleLock(auditor, { permissions: [], builtin: null }, SYSTEM_MODEL)).toMatch(/role templates/);
    expect(assignableRole(auditor, { permissions: ["system.users.manage"], builtin: null }, SYSTEM_MODEL)).toBe(false);
  });
});

describe("adminTabs", () => {
  it("nobody without access sees the panel", () => {
    expect(adminTabs(null)).toEqual([]);
    expect(adminTabs({ permissions: [], is_super_admin: false })).toEqual([]);
  });

  it("shows only the sections the access can view", () => {
    expect(adminTabs({ permissions: ALL, is_super_admin: true })).toEqual(["overview", "projects", "users", "admins", "roles", "audit"]);
    expect(adminTabs({ permissions: ["system.audit.view"], is_super_admin: false })).toEqual(["overview", "audit"]);
  });
});

describe("accountLock mirrors the server's rules", () => {
  it("never your own account, and needs users.manage", () => {
    expect(accountLock(admin, { id: "ad" }, undefined)).toMatch(/own account/);
    expect(accountLock(auditor, { id: "x" }, undefined)).toMatch(/permission/);
    expect(accountLock(admin, { id: "x" }, undefined)).toBeNull();
  });

  it("Super Admins only by a Super Admin; stronger administrators are out of reach", () => {
    expect(accountLock(admin, { id: "sa" }, { permissions: ALL, is_owner: true })).toMatch(/Super Admin/);
    expect(accountLock(superAdmin, { id: "sa2" }, { permissions: ALL, is_owner: true })).toBeNull();
    const manager: Actor = { userId: "m", permissions: ["system.users.view", "system.users.manage"], isOwner: false };
    expect(accountLock(manager, { id: "au" }, { permissions: AUDITOR, is_owner: false })).toMatch(/permissions you do not have/);
  });
});

describe("projectDeleteLock", () => {
  it("the demo project is protected; needs projects.manage", () => {
    expect(projectDeleteLock(admin, { protected: true })).toMatch(/demo/);
    expect(projectDeleteLock(admin, { protected: false })).toBeNull();
    expect(projectDeleteLock(auditor, { protected: false })).toMatch(/permission/);
  });
});

describe("helpers", () => {
  it("canAdmin and systemActor read the session's access", () => {
    const access = { permissions: AUDITOR, is_super_admin: false };
    expect(canAdmin(access, "system.audit.view")).toBe(true);
    expect(canAdmin(access, "system.projects.manage")).toBe(false);
    expect(canAdmin(undefined, "system.audit.view")).toBe(false);
    expect(systemActor("u", { permissions: ALL, is_super_admin: true })).toEqual({ userId: "u", permissions: ALL, isOwner: true });
  });

  it("audit actions read as phrases; unknown ones stay as they are", () => {
    expect(auditActionLabel("admin.project.delete")).toBe("Deleted project");
    expect(auditActionLabel("token.create")).toBe("token.create");
  });
});
