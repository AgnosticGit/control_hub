# Permissions and project roles — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Named permissions and per-project role templates (built-in Owner/Admin/Operator/Viewer plus custom ones), assignable per member as a template or an individual set, managed from a project settings page.

**Architecture:** A pure `permissions` module in `karbird-api` owns the catalog, built-in sets and the anti-escalation rules. PostgreSQL stores templates in `project_roles` (built-ins created by a trigger on `projects`), memberships point to a template or carry an individual set. `authorize()` checks a `Permission` instead of a minimum role. New REST routes for roles and members; the web gets a settings page and gates UI by permissions.

**Tech Stack:** Rust (axum, sqlx, utoipa, proptest), PostgreSQL, Next.js/React, TanStack Query, vitest, node:test, Playwright.

Spec: `docs/superpowers/specs/2026-09-30-permissions-roles-design.md`.

## Global Constraints

- No `unsafe`, no `unwrap`/`expect` on external data; TypeScript strict, no `any`.
- Every mutation: cookie session + allowed Origin + CSRF (existing extractor). API tokens stay read-only (`telemetry.view` only).
- Another tenant's project is 404, never 403.
- After API changes run `just codegen` and keep the generated files.
- Tests at every level (Rust unit/property/handler, vitest, integration, Playwright); never weaken existing tests.
- No commits (the user commits).

## File map

| File | Responsibility |
|---|---|
| `crates/karbird-api/src/permissions.rs` (new) | `Permission` enum, catalog labels, built-in sets, parsing, `can_grant` / anti-escalation checks (pure) |
| `infra/migrations/postgres/0004_roles.sql` (new) | `project_roles`, trigger for built-ins, membership columns, data migration |
| `crates/karbird-storage/src/pg.rs` | membership rows with effective permissions; role/member CRUD |
| `crates/karbird-api/src/auth.rs` | `authorize(..., Permission)`; remove `Role` |
| `crates/karbird-api/src/routes/roles.rs` (new) | roles + members + permissions catalog handlers |
| `crates/karbird-api/src/routes/{session,tokens,actions,telemetry,live,mod}.rs` | permission mapping; `ProjectDto` gains `role_name`, `permissions` |
| `crates/karbird-api/src/{main.rs,openapi.rs,bootstrap.rs,test_fixtures.rs}` | routes, OpenAPI, grants |
| `apps/web/src/lib/permissions.ts` (new) | `can(project, perm)`, labels (pure, vitest) |
| `apps/web/src/components/settings/{SettingsPage,MembersTab,RolesTab}.tsx` (new) | settings UI |
| `apps/web/src/app/app/[projectId]/[environmentId]/settings/page.tsx` (new) | route |
| `apps/web/src/components/{shell/AppShell,tokens/TokensPage,actions/ActionsPage,integration/IntegrationPage}.tsx` | gate by permission |
| `tests/integration/roles.test.mjs`, `tests/e2e/specs/roles.spec.ts` (new) | end-to-end behaviour |

---

### Task 1: Permission model (pure)

**Files:** Create `crates/karbird-api/src/permissions.rs`; modify `crates/karbird-api/src/main.rs` (`mod permissions;`).

**Produces:**
- `enum Permission { TelemetryView, TokensView, TokensManage, ActionsView, ActionsPropose, ActionsApprove, MembersView, MembersManage, RolesManage, ProjectManage }` with `as_str()` (`"telemetry.view"` …), `parse(&str) -> Option<Self>`, `ALL: [Permission; 10]`, `label()`.
- `type PermissionSet = BTreeSet<Permission>`; `parse_set(&[String]) -> Result<PermissionSet, String>` (rejects unknown, used on input); `from_stored(&[String]) -> PermissionSet` (ignores unknown).
- `enum Builtin { Owner, Admin, Operator, Viewer }` with `key()`, `name()`, `permissions()`.
- `struct Actor { permissions: PermissionSet, is_owner: bool, user_id: Uuid }`; `struct Target { user_id: Uuid, is_owner: bool }`.
- `fn check_member_change(actor, target, new: &Grant, owners_left_after: usize) -> Result<(), &'static str>` where `enum Grant { Role { permissions: PermissionSet, builtin: Option<Builtin> }, Individual(PermissionSet), Remove }`.
- `fn check_template_change(actor, old: Option<&PermissionSet>, new: &PermissionSet) -> Result<(), &'static str>`.
- `tokens.manage` implies `tokens.view` (`normalise(set)` adds implied permissions).

- [ ] Step 1: Write unit tests in the module: exact parsing (`"Telemetry.view"`, `" telemetry.view"`, `"*"` rejected); built-in sets match the spec table; `normalise` adds `tokens.view`; `check_member_change` refuses self-edit, superset grants, Owner by non-owner, changing an owner by non-owner, last owner demotion/removal (`owners_left_after == 0`), and allows subset grants; `check_template_change` refuses supersets of the actor for new and existing templates.
- [ ] Step 2: proptest: for random actor sets and random requested sets, `check_member_change` succeeds only if requested ⊆ actor; for random sequences of successful operations over a small member list starting with one owner, no member ever ends up with a permission that the acting member lacked at that step, and at least one owner remains.
- [ ] Step 3: Run `cargo test -p karbird-api permissions` → FAIL (module missing).
- [ ] Step 4: Implement the module.
- [ ] Step 5: Run `cargo test -p karbird-api permissions` → PASS.

### Task 2: Schema and storage

**Files:** Create `infra/migrations/postgres/0004_roles.sql`; modify `crates/karbird-storage/src/pg.rs`, `crates/karbird-api/src/bootstrap.rs` (grants), `crates/karbird-api/src/test_fixtures.rs`.

Migration:

```sql
CREATE TABLE project_roles (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  TEXT NOT NULL REFERENCES projects(id),
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    permissions TEXT[] NOT NULL DEFAULT '{}',
    builtin     TEXT CHECK (builtin IN ('owner', 'admin', 'operator', 'viewer')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX project_roles_name_idx ON project_roles (project_id, lower(name));
CREATE UNIQUE INDEX project_roles_builtin_idx ON project_roles (project_id, builtin) WHERE builtin IS NOT NULL;

CREATE FUNCTION karbird_builtin_roles() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO project_roles (project_id, name, permissions, builtin) VALUES
    (NEW.id, 'Owner', ARRAY['telemetry.view','tokens.view','tokens.manage','actions.view','actions.propose','actions.approve','members.view','members.manage','roles.manage','project.manage'], 'owner'),
    (NEW.id, 'Admin', ARRAY['telemetry.view','tokens.view','tokens.manage','actions.view','actions.propose','actions.approve','members.view','members.manage','roles.manage','project.manage'], 'admin'),
    (NEW.id, 'Operator', ARRAY['telemetry.view','tokens.view','actions.view','actions.propose','members.view'], 'operator'),
    (NEW.id, 'Viewer', ARRAY['telemetry.view','actions.view','members.view'], 'viewer');
  RETURN NEW;
END $$;
CREATE TRIGGER projects_builtin_roles AFTER INSERT ON projects FOR EACH ROW EXECUTE FUNCTION karbird_builtin_roles();
-- existing projects
INSERT INTO project_roles (project_id, name, permissions, builtin)
SELECT p.id, b.name, b.perms, b.key FROM projects p CROSS JOIN (VALUES
  ('Owner', ARRAY['telemetry.view','tokens.view','tokens.manage','actions.view','actions.propose','actions.approve','members.view','members.manage','roles.manage','project.manage'], 'owner'),
  ('Admin', ARRAY['telemetry.view','tokens.view','tokens.manage','actions.view','actions.propose','actions.approve','members.view','members.manage','roles.manage','project.manage'], 'admin'),
  ('Operator', ARRAY['telemetry.view','tokens.view','actions.view','actions.propose','members.view'], 'operator'),
  ('Viewer', ARRAY['telemetry.view','actions.view','members.view'], 'viewer')) AS b(name, perms, key);

ALTER TABLE memberships
  ADD COLUMN role_id UUID REFERENCES project_roles(id),
  ADD COLUMN permissions TEXT[],
  ADD COLUMN based_on UUID REFERENCES project_roles(id) ON DELETE SET NULL,
  ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT now();
UPDATE memberships m SET role_id = r.id FROM project_roles r WHERE r.project_id = m.project_id AND r.builtin = m.role;
ALTER TABLE memberships DROP COLUMN role;
ALTER TABLE memberships ADD CONSTRAINT memberships_one_grant CHECK ((role_id IS NULL) <> (permissions IS NULL));
```

(The trigger function runs as the inserting role, so `karbird_api` needs `INSERT` on `project_roles`.)

Storage (`pg.rs`), **produces:**
- `MembershipRow { tenant_id, project_id, project_name, role_key: Option<String>, role_name: String, permissions: Vec<String>, is_owner: bool }` — `role_key` = builtin key, `role_name` = template name or `"Individual"`, `permissions` = `COALESCE(m.permissions, r.permissions)`.
- `upsert_membership(user_id, project_id, builtin_key: &str)` (resolves the built-in role id).
- `create_account` inserts the owner membership with the project's `owner` role id (after the project insert; the trigger has created it).
- `RoleRow { id: Uuid, name, permissions: Vec<String>, builtin: Option<String>, holders: i64 }`; `roles(project_id)`, `create_role(project_id, name, &[String]) -> Result<Option<Uuid>>` (None on duplicate name), `update_role(project_id, id, name, &[String]) -> Result<bool>` (refuses builtin), `delete_role(project_id, id) -> Result<DeleteRole>` (`Deleted | NotFound | Builtin | Held(i64)`).
- `MemberRow { user_id, email, display_name, role_id: Option<Uuid>, role_name, based_on: Option<Uuid>, permissions: Vec<String>, is_owner: bool, created_at }`; `members(project_id)`, `set_member_role(project_id, user_id, role_id)`, `set_member_permissions(project_id, user_id, &[String], based_on: Option<Uuid>)`, `remove_member(project_id, user_id)` — each returns `bool` (found).
- `count_owners(project_id) -> i64`.
- `delete_users_in_domains` keeps working (memberships have no role column now).

- [ ] Step 1: Add unit tests for pure helpers only (row → effective permissions mapping is SQL; covered by integration).
- [ ] Step 2: Write migration and storage methods; update grants in `bootstrap.rs`: `karbird_api` gets `SELECT, INSERT, UPDATE, DELETE ON project_roles` and `SELECT, INSERT, UPDATE, DELETE ON memberships`; executor `SELECT ON project_roles, memberships` only if used (it is not today, skip).
- [ ] Step 3: `cargo test -p karbird-storage` → PASS; `just bootstrap` applies the migration; `psql` check: built-ins exist for `demo`, owner membership points to the `owner` role.

### Task 3: Authorization by permission

**Files:** modify `auth.rs`, `routes/{telemetry,live,mod,tokens,actions,session}.rs`, `crates/karbird-cli/src/test_http.rs` (fixture shape).

**Consumes:** `Permission`, `from_stored`, `MembershipRow`. **Produces:** `authorize(state, user, project, env, Permission) -> Result<Authorized, ApiError>` where `Authorized { scope: Scope, is_demo: bool, actor: permissions::Actor }`.

Mapping: telemetry/live/coverage/services → `TelemetryView`; token list → `TokensView`; token create/rotate/revoke → `TokensManage`; action catalog/list/get → `ActionsView`; propose → `ActionsPropose`; approve/reject → `ActionsApprove`; cancel → `ActionsPropose` and, when not the proposer, `ActionsApprove`. API tokens: only `TelemetryView`, anything else 403 "API tokens are read-only".

`ProjectDto` adds `role_name: String`, `permissions: Vec<String>`; `role` stays (built-in key, `"custom"` for templates, `"individual"` for individual sets). API-token sessions report `role: "viewer"`, `permissions: ["telemetry.view"]`.

- [ ] Step 1: Replace the `Role` tests in `auth.rs` with: API tokens get 403 for every permission except `TelemetryView`; 404 for other scopes; malformed ids 404 before lookup (keep existing assertions with `Permission::TelemetryView`).
- [ ] Step 2: Implement; update every call site.
- [ ] Step 3: `cargo test -p karbird-api` → PASS; existing handler tests unchanged in meaning.

### Task 4: Roles and members API

**Files:** create `routes/roles.rs`; modify `main.rs` (routes), `openapi.rs`.

Routes:
- `GET /api/v1/permissions` → `[{ id, label, group }]` (session required).
- `GET /api/v1/projects/{p}/roles` (`MembersView`) → `[{ id, name, permissions, builtin, holders }]`.
- `POST /api/v1/projects/{p}/roles` `{ name, permissions }` (`RolesManage`) → 201 `{ id }`; 409 duplicate name; 400 unknown permission / bad name; 403 superset of actor.
- `PATCH /api/v1/projects/{p}/roles/{id}` `{ name, permissions }` → 204; 403 builtin or escalation; 404.
- `DELETE /api/v1/projects/{p}/roles/{id}` → 204; 409 `{ holders }` when held; 403 builtin.
- `GET /api/v1/projects/{p}/members` (`MembersView`) → `[{ user_id, email, display_name, role_id, role_name, based_on, permissions, is_owner, is_me }]`.
- `PATCH /api/v1/projects/{p}/members/{user_id}` `{ role_id } | { permissions, based_on? }` (`MembersManage`) → 204.
- `DELETE /api/v1/projects/{p}/members/{user_id}` (`MembersManage`) → 204.

Project-level routes authorize against the project with any of its environments: add `authorize_project(state, user, project, Permission) -> Result<ProjectAuth { tenant_id, project_id, actor }, ApiError>` in `auth.rs` (404 when not a member; API tokens refused with 403 on these routes).

All mutations write `audit_log` (`action`: `role.create|role.update|role.delete|member.update|member.remove`, before/after JSON without secrets).

- [ ] Step 1: Handler tests (oneshot, fake state where available; otherwise pure checks through `permissions`): 401 without session; 403 without CSRF; 400 unknown permission; 403 self-edit; 403 escalation; 403 last owner; 409 held delete; 404 other tenant.
- [ ] Step 2: Implement handlers and OpenAPI annotations; `just codegen`.
- [ ] Step 3: `cargo test -p karbird-api` → PASS; `just check` → PASS.

### Task 5: Web — permissions helper and gating

**Files:** create `apps/web/src/lib/permissions.ts` + `permissions.test.ts`; modify `AppShell.tsx`, `TokensPage.tsx`, `ActionsPage.tsx`, `IntegrationPage.tsx`, `app/app/page.tsx` (show `role_name`).

**Produces:** `type PermissionId = "telemetry.view" | ...`; `can(project: { permissions: string[] } | undefined, p: PermissionId): boolean`; `PERMISSION_GROUPS` for the matrix (Telemetry, Tokens, Actions, Team, Project).

- [ ] Step 1: vitest: `can` is false for missing project/unknown strings; groups cover every permission exactly once and match the API catalog ids (import from generated schema type).
- [ ] Step 2: Replace `role === "admin" || role === "owner"` with `can(project, "tokens.manage")` etc.; sidebar shows **Settings** when `members.view`.
- [ ] Step 3: `pnpm --filter @karbird/web test` and `typecheck` → PASS.

### Task 6: Web — settings page (Members, Roles)

**Files:** create `apps/web/src/app/app/[projectId]/[environmentId]/settings/page.tsx`, `components/settings/SettingsPage.tsx`, `MembersTab.tsx`, `RolesTab.tsx`, `components/settings/matrix.ts` + `matrix.test.ts` (pure: toggling a box on a template member produces an individual set with `based_on`; whether the actor may edit a row).

UI:
- Tabs `Members` / `Roles` (role="tablist").
- Members: table (Name, Email, Role select, actions). "Fine-tune" opens a panel with grouped checkboxes (labels from `/permissions`); boxes the actor lacks are disabled; saving calls PATCH. Row controls disabled with `title` explaining why (you, owner, lacks permission). Remove with confirm dialog.
- Roles: list of roles with holder counts; built-ins read-only; "New role" (name + checkbox matrix); edit; delete disabled while held ("Held by N members").
- Errors from the API shown in `role="alert"`.

- [ ] Step 1: vitest for `matrix.ts`.
- [ ] Step 2: Implement components with TanStack Query mutations (invalidate `members`, `roles`, `session`).
- [ ] Step 3: `pnpm --filter @karbird/web test`, `just check` → PASS.

### Task 7: Integration and e2e tests, docs

**Files:** create `tests/integration/roles.test.mjs`, `tests/e2e/specs/roles.spec.ts`; update `tests/integration/*` and `tests/e2e/specs/*` that read `role`; `README.md`, `docs/architecture/*` (auth section), `TASKS.md`.

Integration (fixtures: admin=owner of demo, operator, viewer):
- viewer: `/tokens` 403, telemetry 200, members list 200, members PATCH 403.
- admin creates "Manager-<hex>" with `tokens.view,tokens.manage,telemetry.view`; assigns to viewer → viewer can create a token; admin edits template removing `tokens.manage` → viewer 403 again.
- individual set on operator with `actions.view` only → propose 403.
- escalation: operator given `members.manage` (individual) cannot grant `tokens.manage` → 403; cannot edit self → 403.
- last owner: admin cannot demote themselves (self-edit 403) and nobody can remove the only owner (403).
- delete held template 409; after reassigning, 204.
- isolation user: demo roles/members 404; CSRF missing 403; no session 401.
- Clean up: reassign fixture users back to their built-ins and delete created templates (tests independent).

E2E: admin opens Settings → Roles → creates "Manager-<hex>" ticking Telemetry + Tokens → Members → sets viewer to it; viewer (second browser context) sees Tokens in the sidebar and can open it; admin unticks Tokens manage via Fine-tune → viewer's Tokens page shows no create button after reload. Cleanup via API.

- [ ] Step 1: Write both suites; run `node scripts/test-with-fixtures.mjs node --test tests/integration/roles.test.mjs` and the Playwright spec.
- [ ] Step 2: Full `just check`, `just test`, `just test-integration`, `just test-e2e` with `just dev` + `just demo`.
- [ ] Step 3: Docs and TASKS.md.
