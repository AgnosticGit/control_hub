// karbird administrator permissions for the admin panel. The API enforces every rule; these
// mirror them so the UI can hide or disable what would be refused and explain why.
import type { Actor, PermissionModel } from "@/components/settings/matrix";

/** Ids in the API catalog's order (`GET /api/v1/admin/permissions`). */
export const SYSTEM_PERMISSIONS = [
  "system.projects.view",
  "system.projects.manage",
  "system.users.view",
  "system.users.manage",
  "system.team.view",
  "system.team.manage",
  "system.roles.manage",
  "system.audit.view",
] as const;

export type SystemPermissionId = (typeof SYSTEM_PERMISSIONS)[number];

export const SYSTEM_MODEL: PermissionModel = {
  ids: SYSTEM_PERMISSIONS,
  implies: {
    "system.projects.manage": "system.projects.view",
    "system.users.manage": "system.users.view",
    "system.team.manage": "system.team.view",
    "system.roles.manage": "system.team.view",
  },
  membersManage: "system.team.manage",
  rolesManage: "system.roles.manage",
  ownerRole: "super_admin",
  text: {
    own: "You cannot change your own administrator access.",
    needsMembersManage: "Changing administrators needs the “Grant and change administrator access” permission.",
    ownerChangesOwner: "Only a Super Admin can change another Super Admin.",
    strongerMember: "This administrator has permissions you do not have.",
    needsRolesManage: "Editing roles needs the “Manage administrator role templates” permission.",
    strongerRole: "This role has permissions you do not have.",
  },
};

/** The signed-in user's administrator access, from `/session` (`system`). */
export type SystemAccess = { permissions: readonly string[]; is_super_admin: boolean };

export function canAdmin(system: SystemAccess | null | undefined, permission: SystemPermissionId): boolean {
  return system?.permissions.includes(permission) ?? false;
}

export function systemActor(userId: string, system: SystemAccess): Actor {
  return { userId, permissions: system.permissions, isOwner: system.is_super_admin };
}

export type AdminTab = "overview" | "projects" | "users" | "admins" | "roles" | "audit";

export const ADMIN_TAB_LABELS: Record<AdminTab, string> = {
  overview: "Overview",
  projects: "Projects",
  users: "Accounts",
  admins: "Administrators",
  roles: "Roles",
  audit: "Audit log",
};

/** Tabs the access can open, in display order. */
export function adminTabs(system: SystemAccess | null | undefined): AdminTab[] {
  if (!system || system.permissions.length === 0) return [];
  const tabs: AdminTab[] = ["overview"];
  if (canAdmin(system, "system.projects.view")) tabs.push("projects");
  if (canAdmin(system, "system.users.view")) tabs.push("users");
  if (canAdmin(system, "system.team.view")) tabs.push("admins", "roles");
  if (canAdmin(system, "system.audit.view")) tabs.push("audit");
  return tabs;
}

/** Why `actor` may not disable, enable or sign out this account, or null if they may. `admin` is
 *  the account's administrator grant, if it has one. */
export function accountLock(
  actor: Actor,
  account: { id: string },
  admin: { permissions: readonly string[]; is_owner: boolean } | undefined,
): string | null {
  if (account.id === actor.userId) return "Change your own account in your account settings.";
  if (!actor.permissions.includes("system.users.manage")) return "Needs the “Disable and enable accounts” permission.";
  if (admin?.is_owner && !actor.isOwner) return "Only a Super Admin can change a Super Admin's account.";
  if (admin && !actor.isOwner && !admin.permissions.every((p) => actor.permissions.includes(p))) {
    return "This administrator has permissions you do not have.";
  }
  return null;
}

export function projectDeleteLock(actor: Actor, project: { protected: boolean }): string | null {
  if (!actor.permissions.includes("system.projects.manage")) return "Needs the “Create, rename and delete projects” permission.";
  if (project.protected) return "The demo project cannot be deleted.";
  return null;
}

/** Audit actions as short phrases for people; unknown actions are shown as they are. */
export function auditActionLabel(action: string): string {
  const known: Record<string, string> = {
    "admin.project.create": "Created project",
    "admin.project.rename": "Renamed project",
    "admin.project.delete": "Deleted project",
    "admin.environment.create": "Added environment",
    "admin.member.put": "Set project member",
    "admin.member.remove": "Removed project member",
    "admin.user.disable": "Disabled account",
    "admin.user.enable": "Enabled account",
    "admin.user.sessions_revoke": "Ended account sessions",
    "admin.admin.add": "Added administrator",
    "admin.admin.update": "Changed administrator",
    "admin.admin.remove": "Removed administrator",
    "admin.role.create": "Created administrator role",
    "admin.role.update": "Changed administrator role",
    "admin.role.delete": "Deleted administrator role",
  };
  return known[action] ?? action;
}
