import { describe, expect, it } from "vitest";
import { PERMISSIONS } from "@/lib/permissions";
import { assignableRole, defaultInviteRole, groupCatalog, memberAfter, memberLock, normalise, roleLock, toggle, type Actor } from "./matrix";

const ALL = [...PERMISSIONS];
const owner: Actor = { userId: "u-owner", permissions: ALL, isOwner: true };
const admin: Actor = { userId: "u-admin", permissions: ALL, isOwner: false };
const manager: Actor = { userId: "u-manager", permissions: ["telemetry.view", "members.view", "members.manage"], isOwner: false };

describe("defaultInviteRole", () => {
  const roles = [
    { id: "o", permissions: ALL, builtin: "owner" },
    { id: "a", permissions: ALL, builtin: "admin" },
    { id: "v", permissions: ["telemetry.view", "actions.view", "members.view"], builtin: "viewer" },
    { id: "t", permissions: ["telemetry.view"], builtin: null },
  ];

  it("prefers Viewer", () => {
    expect(defaultInviteRole(owner, roles)?.id).toBe("v");
  });

  it("falls back to the first role the actor may grant, never one they may not", () => {
    expect(defaultInviteRole(manager, roles)?.id).toBe("t");
    expect(defaultInviteRole(manager, roles.slice(0, 2))).toBeUndefined();
  });
});

describe("normalise", () => {
  it("adds implied permissions, drops unknown ids and keeps catalog order", () => {
    expect(normalise(["tokens.manage", "telemetry.view", "root"])).toEqual(["telemetry.view", "tokens.view", "tokens.manage"]);
    expect(normalise(["roles.manage"])).toEqual(["members.view", "roles.manage"]);
    expect(normalise([])).toEqual([]);
  });
});

describe("toggle", () => {
  it("ticking a manage permission ticks what it needs", () => {
    expect(toggle(["telemetry.view"], "actions.approve", true)).toEqual(["telemetry.view", "actions.view", "actions.approve"]);
  });

  it("unticking a view permission unticks everything that needs it", () => {
    const set = ["actions.view", "actions.propose", "actions.approve", "telemetry.view"];
    expect(toggle(set, "actions.view", false)).toEqual(["telemetry.view"]);
    expect(toggle(["members.view", "members.manage", "roles.manage"], "members.view", false)).toEqual([]);
  });

  it("unticking a manage permission keeps the view", () => {
    expect(toggle(["tokens.view", "tokens.manage"], "tokens.manage", false)).toEqual(["tokens.view"]);
  });
});

describe("groupCatalog", () => {
  it("groups entries in first-seen order", () => {
    const g = groupCatalog([
      { id: "telemetry.view", label: "T", group: "Telemetry" },
      { id: "tokens.view", label: "TV", group: "Tokens" },
      { id: "tokens.manage", label: "TM", group: "Tokens" },
    ]);
    expect(g.map((x) => [x.group, x.items.map((i) => i.id)])).toEqual([
      ["Telemetry", ["telemetry.view"]],
      ["Tokens", ["tokens.view", "tokens.manage"]],
    ]);
  });
});

const member = (userId: string, permissions: string[], isOwner = false) => ({ user_id: userId, permissions, is_owner: isOwner });

describe("memberLock mirrors the server's rules", () => {
  it("nobody edits themselves", () => {
    expect(memberLock(owner, member("u-owner", ALL, true))).toMatch(/your own/);
  });

  it("needs members.manage", () => {
    const viewer: Actor = { userId: "v", permissions: ["telemetry.view", "members.view"], isOwner: false };
    expect(memberLock(viewer, member("x", []))).toMatch(/permission/);
  });

  it("only owners change owners", () => {
    expect(memberLock(admin, member("o", ALL, true))).toMatch(/owner/i);
    expect(memberLock(owner, member("o2", ALL, true))).toBeNull();
  });

  it("members with more permissions than the actor are out of reach", () => {
    expect(memberLock(manager, member("a", ALL))).toMatch(/permissions you do not have/);
    expect(memberLock(manager, member("v", ["telemetry.view"]))).toBeNull();
    expect(memberLock(owner, member("a", ALL))).toBeNull();
  });
});

describe("assignableRole", () => {
  it("only roles within the actor's permissions; Owner only by an owner", () => {
    const viewerRole = { permissions: ["telemetry.view", "actions.view", "members.view"], builtin: "viewer" };
    const ownerRole = { permissions: ALL, builtin: "owner" };
    expect(assignableRole(manager, { permissions: ["telemetry.view"], builtin: null })).toBe(true);
    expect(assignableRole(manager, viewerRole)).toBe(false);
    expect(assignableRole(admin, ownerRole)).toBe(false);
    expect(assignableRole(owner, ownerRole)).toBe(true);
  });
});

describe("memberAfter", () => {
  const roles = [{ id: "r1", name: "Manager", permissions: ["telemetry.view", "tokens.view"] }];

  it("a role replaces the individual set", () => {
    expect(memberAfter({ role_id: "r1" }, roles)).toEqual({
      role_id: "r1",
      role_name: "Manager",
      permissions: ["telemetry.view", "tokens.view"],
      based_on: null,
    });
  });

  it("an individual set drops the role, keeps what it was based on and adds implied permissions", () => {
    expect(memberAfter({ permissions: ["tokens.manage"], based_on: "r1" }, roles)).toEqual({
      role_id: null,
      role_name: "Individual",
      permissions: ["tokens.view", "tokens.manage"],
      based_on: "r1",
    });
  });

  it("an unknown role changes nothing locally (the server answers)", () => {
    expect(memberAfter({ role_id: "missing" }, roles)).toEqual({});
  });
});

describe("roleLock", () => {
  const rolesManager: Actor = { userId: "r", permissions: ["telemetry.view", "members.view", "roles.manage"], isOwner: false };

  it("built-in roles are read-only for everyone", () => {
    expect(roleLock(owner, { permissions: ALL, builtin: "admin" })).toMatch(/Built-in/);
  });

  it("needs roles.manage and cannot touch stronger roles", () => {
    expect(roleLock(manager, { permissions: [], builtin: null })).toMatch(/permission/);
    expect(roleLock(rolesManager, { permissions: ["telemetry.view"], builtin: null })).toBeNull();
    expect(roleLock(rolesManager, { permissions: ["tokens.view"], builtin: null })).toMatch(/permissions you do not have/);
  });
});
