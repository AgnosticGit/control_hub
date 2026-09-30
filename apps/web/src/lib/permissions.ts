/** Permission ids, in the API catalog's order (`GET /api/v1/permissions`). */
export const PERMISSIONS = [
  "telemetry.view",
  "tokens.view",
  "tokens.manage",
  "actions.view",
  "actions.propose",
  "actions.approve",
  "members.view",
  "members.manage",
  "roles.manage",
  "project.manage",
] as const;

export type PermissionId = (typeof PERMISSIONS)[number];

/** The effective permissions of the signed-in user in one project (from `/session`). */
export type ProjectGrant = {
  permissions: readonly string[];
  role: string;
  role_name: string;
};

/** UI gating only; the API enforces every permission itself. */
export function can(project: Pick<ProjectGrant, "permissions"> | null | undefined, permission: PermissionId): boolean {
  return project?.permissions.includes(permission) ?? false;
}

/** Approve or reject: never your own plan. */
export function canDecidePlan(project: Pick<ProjectGrant, "permissions"> | null | undefined, mine: boolean): boolean {
  return !mine && can(project, "actions.approve");
}

export function canCancelPlan(project: Pick<ProjectGrant, "permissions"> | null | undefined, mine: boolean): boolean {
  return can(project, mine ? "actions.propose" : "actions.approve");
}

export function roleLabel(project: Pick<ProjectGrant, "role_name"> | null | undefined): string {
  return project?.role_name ?? "";
}
