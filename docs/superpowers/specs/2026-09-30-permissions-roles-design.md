# Permissions and project roles — design

Part 1 of 4 (then: members and invite links, personal account, onboarding). Approved 2026-09-30.

## Goal

Replace the fixed role ladder (`viewer < operator < admin < owner`) with named permissions and
per-project role templates. A project admin sees the members of a project, picks a role for each or
fine-tunes their permissions with checkboxes, and creates custom templates such as "Manager".

## Permission catalog

Fixed in code (`karbird-api`), project-wide (all environments of the project):

| Permission | Allows |
|---|---|
| `telemetry.view` | map, timeline, traces, logs, events, live channel, coverage, services, topology |
| `tokens.view` | list tokens (metadata only) |
| `tokens.manage` | create / rotate / revoke ingestion and API tokens (implies `tokens.view`) |
| `actions.view` | list and read action plans, the action catalog |
| `actions.propose` | propose a plan, cancel one's own plan |
| `actions.approve` | approve / reject others' plans, cancel any plan |
| `members.view` | list project members |
| `members.manage` | change members' roles, remove members (invites: part 2) |
| `roles.manage` | create / edit / delete role templates |
| `project.manage` | reserved for project settings (rename, environments) |

Unknown permission strings are rejected on input and ignored (never granted) when read from storage.

## Built-in roles

Created in every project, not editable, not deletable:

- **Owner** — every permission, plus the owner flag: only owners can grant or revoke Owner.
- **Admin** — every permission.
- **Operator** — `telemetry.view`, `tokens.view`, `actions.view`, `actions.propose`, `members.view`.
- **Viewer** — `telemetry.view`, `actions.view`, `members.view`.

Existing memberships are migrated onto these by their current role name.

## Custom templates

Per project: a name (1–64 chars, unique per project, case-insensitive) and a set of permissions.
Editing a template changes the effective permissions of every member who holds it at once. A template
that is still held cannot be deleted (409 with the number of holders).

## Member role

Exactly one of: a role template (`role_id`) or an individual permission set (`permissions`), enforced by
a `CHECK`. In the UI, ticking a box on a member who holds a template switches them to an individual set
("Individual, based on Manager"); `based_on` is kept for display only.

## Anti-escalation rules (server-side)

1. Nobody changes their own membership.
2. The permissions granted (template or individual set) must be a subset of the actor's own permissions.
3. Editing or creating a template: its permissions must be a subset of the actor's; editing a template the
   actor could not grant (superset of theirs) is refused.
4. Only an owner assigns or removes the Owner role, or changes an owner's membership.
5. The last owner of a project cannot be demoted or removed (re-checked inside the storage transaction
   under a per-project lock, so concurrent demotions cannot leave a project without owners).
5a. A non-owner cannot change or remove a member who has permissions the actor lacks.
6. A plan is never approved by its proposer, whatever the permissions.

Violations return 403 with a short reason; another tenant's project stays 404.

## Storage (PostgreSQL migration `0004_roles.sql`)

- `project_roles(id uuid pk, project_id fk, name, permissions text[], builtin text null, created_at)`;
  `builtin` in (`owner`,`admin`,`operator`,`viewer`) for built-ins; unique `(project_id, lower(name))`.
- `memberships`: add `role_id uuid null fk project_roles`, `permissions text[] null`, `based_on uuid null`;
  migrate `role` → `role_id`; drop `role`; `CHECK ((role_id IS NULL) <> (permissions IS NULL))`.
- Built-ins are inserted for existing projects by the migration and for new projects by sign-up and bootstrap.
- `karbird_api` gets `SELECT, INSERT, UPDATE, DELETE` on `project_roles` and `UPDATE, DELETE` on
  `memberships`; the executor keeps read access.
- Every role/member change writes `audit_log` (actor, project, before/after, outcome).

## API

- `authorize(...)` takes a `Permission` instead of a minimum `Role`; all existing routes are mapped
  (telemetry → `telemetry.view`, token list → `tokens.view`, token mutations → `tokens.manage`, action
  reads → `actions.view`, propose/cancel own → `actions.propose`, approve/reject/cancel others →
  `actions.approve`). API tokens (read-only, one environment) keep at most `telemetry.view` and `actions.view` (the CLI/MCP
  reads plans), intersected with the token owner's permissions.
- `GET /projects/{p}/roles`, `POST /projects/{p}/roles`, `PATCH /projects/{p}/roles/{id}`,
  `DELETE /projects/{p}/roles/{id}` — `roles.manage` for mutations, `members.view` to list.
- `GET /projects/{p}/members` (`members.view`), `PATCH /projects/{p}/members/{user_id}` with
  `{ role_id }` or `{ permissions }`, `DELETE /projects/{p}/members/{user_id}` (`members.manage`).
- `GET /permissions` — the catalog with labels, for the UI.
- `/session` projects carry `role` (display name) and `permissions` (effective).
- Mutations: cookie session + Origin + CSRF, as today. OpenAPI and generated client via `just codegen`.

## Web

- Project settings page `/app/{project}/{env}/settings` with tabs **Members** and **Roles** (sidebar entry
  when the user has `members.view`).
- Members: table (name, email, role select, "Fine-tune" opens permission checkboxes), remove button;
  controls disabled with a tooltip when the actor may not change that member.
- Roles: built-ins (read-only) and custom templates as a permission matrix with checkboxes; create,
  rename, delete (disabled while held, shows holder count).
- Existing UI gating (tokens, actions buttons) switches from role names to permissions.

## Tests

- Rust unit: catalog parsing, built-in sets, effective permission resolution, subset rules; proptest:
  random sequences of role/member operations never give any member a permission the acting member lacked.
- Rust handler (oneshot): 401 without session, 403 without CSRF / permission, last owner, self-edit,
  escalation, 404 for another tenant, 409 deleting a held template.
- Integration (`tests/integration/roles.test.mjs`): create "Manager", assign, verify token/action access
  follows, edit template → access changes, individual set, isolation between tenants.
- E2E (`tests/e2e/specs/roles.spec.ts`): admin creates "Manager" with checkboxes, assigns it to a member;
  the member sees the allowed pages and not the others.

## Out of scope (later parts)

Invitations and invite links (part 2), personal account pages (part 3), onboarding (part 4),
environment-level permissions, organisation-level templates.
