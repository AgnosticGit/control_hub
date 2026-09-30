// Permission-matrix logic for the Members & roles page. The API enforces every rule; these
// mirror them so the UI can disable what would be refused and explain why.
import { PERMISSIONS, type PermissionId } from "@/lib/permissions";

/** Permissions another one cannot work without (same table as the API). */
const IMPLIES: Partial<Record<PermissionId, PermissionId>> = {
  "tokens.manage": "tokens.view",
  "actions.propose": "actions.view",
  "actions.approve": "actions.view",
  "members.manage": "members.view",
  "roles.manage": "members.view",
};

function isPermission(id: string): id is PermissionId {
  return (PERMISSIONS as readonly string[]).includes(id);
}

/** Known ids plus what they imply, in catalog order. */
export function normalise(ids: Iterable<string>): PermissionId[] {
  const set = new Set<PermissionId>();
  for (const id of ids) {
    if (!isPermission(id)) continue;
    set.add(id);
    const implied = IMPLIES[id];
    if (implied) set.add(implied);
  }
  return PERMISSIONS.filter((p) => set.has(p));
}

/** Ticking adds what the permission needs; unticking removes what needs it. */
export function toggle(current: readonly string[], id: PermissionId, on: boolean): PermissionId[] {
  if (on) return normalise([...current, id]);
  return normalise(current).filter((p) => p !== id && IMPLIES[p] !== id);
}

export type CatalogEntry = { id: string; label: string; group: string };

export function groupCatalog(catalog: readonly CatalogEntry[]): Array<{ group: string; items: CatalogEntry[] }> {
  const groups: Array<{ group: string; items: CatalogEntry[] }> = [];
  for (const entry of catalog) {
    const g = groups.find((x) => x.group === entry.group);
    if (g) g.items.push(entry);
    else groups.push({ group: entry.group, items: [entry] });
  }
  return groups;
}

export type Actor = { userId: string; permissions: readonly string[]; isOwner: boolean };

function subset(a: readonly string[], b: readonly string[]): boolean {
  return a.every((p) => b.includes(p));
}

/** Why `actor` may not change this member, or null if they may. */
export function memberLock(actor: Actor, member: { user_id: string; permissions: readonly string[]; is_owner: boolean }): string | null {
  if (member.user_id === actor.userId) return "You cannot change your own membership.";
  if (!actor.permissions.includes("members.manage")) return "Changing members needs the “Manage members and their roles” permission.";
  if (member.is_owner && !actor.isOwner) return "Only an owner can change another owner.";
  if (!actor.isOwner && !subset(member.permissions, actor.permissions)) {
    return "This member has permissions you do not have.";
  }
  return null;
}

export function assignableRole(actor: Actor, role: { permissions: readonly string[]; builtin?: string | null }): boolean {
  if (role.builtin === "owner" && !actor.isOwner) return false;
  return subset(normalise(role.permissions), actor.permissions);
}

/** The role a new invite link starts with: Viewer if the actor may grant it, else the first they may. */
export function defaultInviteRole<R extends { permissions: readonly string[]; builtin?: string | null }>(
  actor: Actor,
  roles: readonly R[],
): R | undefined {
  const allowed = roles.filter((r) => assignableRole(actor, r));
  return allowed.find((r) => r.builtin === "viewer") ?? allowed[0];
}

type MemberGrant = { role_id: string | null; role_name: string; permissions: string[]; based_on: string | null };

/** How a member looks after `change`, shown until the server's answer arrives. */
export function memberAfter(
  change: { role_id?: string | null; permissions?: string[] | null; based_on?: string | null },
  roles: ReadonlyArray<{ id: string; name: string; permissions: string[] }>,
): Partial<MemberGrant> {
  if (change.role_id) {
    const role = roles.find((r) => r.id === change.role_id);
    return role ? { role_id: role.id, role_name: role.name, permissions: role.permissions, based_on: null } : {};
  }
  if (change.permissions) {
    return { role_id: null, role_name: "Individual", permissions: normalise(change.permissions), based_on: change.based_on ?? null };
  }
  return {};
}

/** Why `actor` may not edit or delete this role, or null if they may. */
export function roleLock(actor: Actor, role: { permissions: readonly string[]; builtin?: string | null }): string | null {
  if (role.builtin) return "Built-in roles are read-only.";
  if (!actor.permissions.includes("roles.manage")) return "Editing roles needs the “Manage role templates” permission.";
  if (!subset(normalise(role.permissions), actor.permissions)) return "This role has permissions you do not have.";
  return null;
}
