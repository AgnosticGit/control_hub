# Organisation, Project Management, OAuth and Email Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn Karbird into an organisation-based observability workspace with project-scoped access, multi-environment setup, Google sign-in and Resend invitations.

**Architecture:** Preserve the existing telemetry scope by treating the current tenant ID as the organisation ID. Add organisation membership beside project membership, then gate project access on both records. Keep OAuth, Resend and invitation delivery in the Rust API as server-side adapters; Next.js only starts browser flows and renders account management.

**Tech Stack:** Rust 2024, Axum, sqlx/PostgreSQL, OpenAPI/ts-rs generation, Next.js 16, React Query, Vitest, Playwright, Resend HTTP API, Google OpenID Connect.

**Spec:** `docs/superpowers/specs/2026-10-07-organisation-project-management-design.md`

## Global Constraints

- No telemetry, token, archive or ClickHouse scope migration: current `tenant_id` is the organisation ID.
- A person must belong to an organisation and have an explicit project assignment before seeing that project's data.
- Project roles apply to every environment in that project; environment-specific roles are out of scope.
- OAuth uses Authorization Code with PKCE, one-time expiring state, verified Google ID-token claims and `email_verified` only.
- Existing verified account email links automatically to Google; no duplicate account may be created.
- Resend, Google credentials and invite secrets are server-side only; never log, return, commit or expose them to browser code.
- Generated API contracts must be refreshed with `just codegen`.
- Do not commit or push. Leave all changes in the working tree for the user.
- Before completion run `just check`, `just test`, `just test-integration` and `just test-e2e` with `just dev` and `just demo` running.

## Review Focus

- An organisation member without a project assignment receives 404 for every project and environment endpoint; cover in Task 3 integration tests.
- A Google callback with wrong `state`, missing PKCE proof, wrong issuer/audience or unverified email never starts a session; cover in Task 6 handler tests.
- A pre-existing password account and a first Google sign-in using the same verified email resolve to one user; cover in Task 6 integration tests.
- An invitation cannot be accepted from a different email or after expiry, and no raw invite secret reaches a list or audit response; cover in Task 5 integration tests.
- Project creation with custom environment names is atomic: duplicate/invalid names leave no organisation/project/environment rows; cover in Task 4 handler and integration tests.

---

## File structure

| File | Responsibility |
|---|---|
| `infra/migrations/postgres/0011_organisations.sql` | Organisation/membership/OAuth/invite schema and migration from existing memberships. |
| `crates/karbird-storage/src/pg.rs` | Scoped organisation, project, environment, identity and invite persistence methods. |
| `crates/karbird-api/src/auth.rs` | Organisation + project authorization primitives. |
| `crates/karbird-api/src/routes/organisations.rs` | Organisation members, project assignments and organisation invitation routes. |
| `crates/karbird-api/src/routes/account.rs` | Atomic first-run and multi-environment project creation. |
| `crates/karbird-api/src/routes/oauth.rs` | Google OAuth state, PKCE, verified identity exchange and session hand-off. |
| `crates/karbird-api/src/mail.rs` | Resend client trait, payload validation and no-op/test transports. |
| `crates/karbird-api/src/state.rs`, `main.rs` | Server-side configuration and routing. |
| `deploy/compose.app.yaml`, `scripts/deploy-config.mjs`, `.github/workflows/deploy.yml`, `.env.example` | Server-only production configuration wiring. |
| `apps/web/src/components/projects/*` | Project hub, first-run wizard and project/environment management. |
| `apps/web/src/components/organisation/*` | People, project assignment and invitations UI. |
| `apps/web/src/app/app/**` | Routes and navigation for the new hub and settings. |

## Tasks

### Task 1: Organisation schema and safe migration

**Files:**
- Create: `infra/migrations/postgres/0011_organisations.sql`
- Modify: `crates/karbird-storage/src/pg.rs`
- Test: `crates/karbird-storage/src/pg.rs`
- Test: `tests/integration/migrate.test.mjs`

**Interfaces:**
- Produces `OrganizationRow`, `OrganizationMemberRow`, `ProjectAssignmentRow` and `PgStore::organisation_for_project(&str) -> Result<Option<OrganizationRow>>`.
- Existing tenant IDs remain valid organisation IDs for every downstream `Scope`.

- [ ] **Step 1: Write failing migration and storage tests**

Add tests that assert migration creates organisation membership for every existing project membership, preserves tenant/project/environment IDs, and is idempotent.

- [ ] **Step 2: Run the migration/storage tests to verify failure**

Run: `cargo test -p karbird-storage organisation`

Expected: FAIL because organisation tables and persistence methods do not exist.

- [ ] **Step 3: Add migration `0011_organisations.sql` and storage rows/methods**

Create `organisations`, `organisation_memberships`, `project_memberships`, `oauth_identities` and `organisation_invites`. Backfill each tenant to an organisation and each existing membership to both membership tables in one transaction. Preserve prior project role IDs/permissions instead of re-encoding roles.

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p karbird-storage organisation`

Expected: PASS.

- [ ] **Step 5: Run real migration integration test**

Run: `node tests/integration/migrate.test.mjs`

Expected: PASS and existing scope IDs unchanged.

### Task 2: Organisation-aware authorization and session contract

**Files:**
- Modify: `crates/karbird-api/src/auth.rs`
- Modify: `crates/karbird-api/src/routes/session.rs`
- Modify: `crates/karbird-api/src/permissions.rs`
- Modify: `packages/api-client/src/generated/schema.ts`
- Modify: `schemas/generated/openapi.json`
- Test: `crates/karbird-api/src/auth.rs`
- Test: `tests/integration/roles.test.mjs`

**Interfaces:**
- Produces `OrganizationDto`, `OrganizationMembershipDto`, and `ProjectDto { organization_id, ... }`.
- Produces `authorize_organization(state, user, organization_id, need)` and strengthens `authorize_project` to require an organisation membership as well as a project assignment.

- [ ] **Step 1: Write failing authorization tests**

Assert an organisation member without a project assignment receives 404 for project and environment access, while a valid assignment retains the previous permissions.

- [ ] **Step 2: Run the focused tests to verify failure**

Run: `cargo test -p karbird-api organization_access`

Expected: FAIL because authorization reads only project memberships.

- [ ] **Step 3: Implement organisation membership checks and session DTOs**

Use `organization_memberships` and `project_memberships` for browser sessions. API tokens remain fixed to their existing `Scope` and must still not cross project/environment boundaries.

- [ ] **Step 4: Regenerate contracts and run tests**

Run: `just codegen; cargo test -p karbird-api organization_access; node tests/integration/roles.test.mjs`

Expected: PASS.

### Task 3: Atomic organisation/project/environment creation

**Files:**
- Modify: `crates/karbird-storage/src/pg.rs`
- Modify: `crates/karbird-api/src/routes/account.rs`
- Modify: `crates/karbird-api/src/routes/signup.rs`
- Modify: `crates/karbird-api/src/main.rs`
- Test: `crates/karbird-api/src/routes/account.rs`
- Test: `tests/integration/account.test.mjs`

**Interfaces:**
- Replaces `ProjectRequest` with `CreateProjectRequest { organization_id: String, name: String, environments: Vec<EnvironmentInput> }`.
- Adds `FirstRunRequest { organization_name, project_name, environments }` and `POST /api/v1/onboarding/first-project`.
- Produces `PgStore::create_project_with_environments(user_id, organisation_id, name, environments) -> Result<ProjectDtoSource>`.

- [ ] **Step 1: Write failing route and integration tests**

Cover a first-run request creating organisation, owner membership, project and `Development`, `Staging`, `Production`; a later project with custom environments; invalid or duplicate environment names rolling back the entire request.

- [ ] **Step 2: Run focused tests to verify failure**

Run: `cargo test -p karbird-api create_project_with_environments; node tests/integration/account.test.mjs`

Expected: FAIL because the current request creates only `production` and no organisation.

- [ ] **Step 3: Implement the transactional creation API**

Validate names with existing `required_name`; enforce unique environment names case-insensitively per project; create the creator's organisation owner and project owner records atomically; write audit events without request secrets.

- [ ] **Step 4: Regenerate and verify**

Run: `just codegen; cargo test -p karbird-api create_project_with_environments; node tests/integration/account.test.mjs`

Expected: PASS.

### Task 4: Organisation people, project assignment and invitations

**Files:**
- Create: `crates/karbird-api/src/routes/organisations.rs`
- Modify: `crates/karbird-api/src/routes/invites.rs`
- Modify: `crates/karbird-api/src/main.rs`
- Modify: `crates/karbird-storage/src/pg.rs`
- Test: `crates/karbird-api/src/routes/organisations.rs`
- Test: `tests/integration/invites.test.mjs`

**Interfaces:**
- Adds `GET/POST /api/v1/organizations/{organization_id}/members`.
- Adds `PUT/DELETE /api/v1/organizations/{organization_id}/projects/{project_id}/members/{user_id}`.
- Adds organisation invite create/list/revoke/accept DTOs with project-role assignments.

- [ ] **Step 1: Write failing tests for invite and assignment boundaries**

Assert invite acceptance matches recipient email, expired and revoked invites fail, accepted people initially see only assigned projects, and an admin cannot assign a role beyond their own permission ceiling.

- [ ] **Step 2: Run focused tests to verify failure**

Run: `cargo test -p karbird-api organization_invite; node tests/integration/invites.test.mjs`

Expected: FAIL because invitations are project-only.

- [ ] **Step 3: Implement organisation routes and adapt invite flow**

Hash invite secrets using existing secret utilities; retain a project-invite compatibility route only when it delegates to the organisation implementation; audit every membership and assignment change.

- [ ] **Step 4: Regenerate and verify**

Run: `just codegen; cargo test -p karbird-api organization_invite; node tests/integration/invites.test.mjs`

Expected: PASS.

### Task 5: Mail abstraction and Resend delivery

**Files:**
- Create: `crates/karbird-api/src/mail.rs`
- Modify: `crates/karbird-api/src/state.rs`
- Modify: `crates/karbird-api/src/main.rs`
- Modify: `crates/karbird-api/src/routes/organisations.rs`
- Test: `crates/karbird-api/src/mail.rs`
- Test: `tests/integration/invites.test.mjs`

**Interfaces:**
- Produces `trait Mailer { async fn send_invite(&self, message: InviteEmail) -> Result<(), MailError>; }`.
- Produces `ResendMailer` and `DisabledMailer`; `AppState` stores `Arc<dyn Mailer>`.

- [ ] **Step 1: Write failing mail tests**

Assert Resend requests use HTTPS, `Authorization: Bearer`, configured sender, recipient, safe invitation URL and no secret appears in logs or list DTOs. Assert disabled mail preserves an invitation as pending and returns an operator-visible delivery failure.

- [ ] **Step 2: Run focused tests to verify failure**

Run: `cargo test -p karbird-api resend`

Expected: FAIL because no mail adapter exists.

- [ ] **Step 3: Implement Resend adapter and route integration**

Use a direct HTTP client with bounded timeout. Keep `KARBIRD_RESEND_API_KEY` and `KARBIRD_MAIL_FROM` optional: production startup fails only when mail delivery is explicitly enabled without both values.

- [ ] **Step 4: Run tests**

Run: `cargo test -p karbird-api resend; node tests/integration/invites.test.mjs`

Expected: PASS.

### Task 6: Google OAuth with PKCE and account linking

**Files:**
- Create: `crates/karbird-api/src/routes/oauth.rs`
- Modify: `crates/karbird-api/src/state.rs`
- Modify: `crates/karbird-api/src/main.rs`
- Modify: `crates/karbird-storage/src/pg.rs`
- Modify: `apps/web/src/app/app/login/page.tsx`
- Modify: `apps/web/src/app/app/register/page.tsx`
- Test: `crates/karbird-api/src/routes/oauth.rs`
- Test: `tests/integration/signup.test.mjs`
- Test: `tests/e2e/specs/owner-login.spec.ts`

**Interfaces:**
- Adds `GET /api/v1/auth/google/start?next=<safe app path>` and `GET /api/v1/auth/google/callback`.
- Produces `GoogleOAuthConfig`, `OAuthStateStore` and `PgStore::link_google_identity(user_id, subject, email)`.

- [ ] **Step 1: Write failing handler tests**

Cover state replay, expired state, PKCE mismatch, invalid issuer/audience/signature, false `email_verified`, new user, existing email/password user and already-linked identity.

- [ ] **Step 2: Run focused tests to verify failure**

Run: `cargo test -p karbird-api google_oauth`

Expected: FAIL because routes and verifier do not exist.

- [ ] **Step 3: Implement standards-compliant OAuth flow**

Use an OpenID Connect client with discovery/JWK verification and Authorization Code + PKCE. Store only a hash of the one-time state and verifier, with a short expiry. On successful verified email, link existing account by email or create an account with no password, then call existing `start_session`.

- [ ] **Step 4: Add login/register controls and verify the browser flow**

Use a normal navigation to `/api/v1/auth/google/start`; preserve only `safeNext` paths. Do not expose client secrets or provider token data to React.

- [ ] **Step 5: Run tests**

Run: `cargo test -p karbird-api google_oauth; node tests/integration/signup.test.mjs; pnpm --filter @karbird/web test; node scripts/test-with-fixtures.mjs pnpm --filter @karbird/e2e test --grep "Google|sign in"`

Expected: PASS.

### Task 7: Project hub, organisation settings and creation wizard

**Files:**
- Create: `apps/web/src/components/projects/ProjectHub.tsx`
- Create: `apps/web/src/components/projects/ProjectWizard.tsx`
- Create: `apps/web/src/components/organisation/PeoplePage.tsx`
- Create: `apps/web/src/components/organisation/ProjectSettingsPage.tsx`
- Modify: `apps/web/src/app/app/page.tsx`
- Modify: `apps/web/src/components/account/ProjectsPage.tsx`
- Modify: `apps/web/src/components/account/AccountShell.tsx`
- Modify: `apps/web/src/lib/api.ts`
- Test: `apps/web/src/components/projects/ProjectWizard.test.tsx`
- Test: `apps/web/src/components/projects/ProjectHub.test.tsx`
- Test: `tests/e2e/specs/projects.spec.ts`

**Interfaces:**
- `ProjectWizard` consumes `CreateOrganizationAndProjectRequest` for first run and `CreateProjectRequest` for later creation.
- `ProjectHub` consumes `SessionResponse.organizations` and selected organisation ID.

- [ ] **Step 1: Write failing UI and E2E tests**

Cover a first-run wizard with three defaults, removing/renaming/adding an environment, project hub showing only assigned projects, a member with no project seeing no project card, and changing environment from the workspace selector.

- [ ] **Step 2: Run tests to verify failure**

Run: `pnpm --filter @karbird/web test -- ProjectWizard ProjectHub; node scripts/test-with-fixtures.mjs pnpm --filter @karbird/e2e test --grep "project hub|first project|environment"`

Expected: FAIL because `/app` is currently a flat environment chooser.

- [ ] **Step 3: Implement the project-first interface**

Replace auto-redirect from `/app` with ProjectHub. Make environment state visible on each project card. Reuse `apiSentence`, accessible forms and React Query cache invalidation. Keep the table/map fallback working without WebGL.

- [ ] **Step 4: Implement organisation people and project settings screens**

Use role/permission driven controls, assignment UI and invitation status. Avoid browser prompts and confirms; use accessible dialogs/forms with explicit result states.

- [ ] **Step 5: Run focused web and browser tests**

Run: `pnpm --filter @karbird/web test; node scripts/test-with-fixtures.mjs pnpm --filter @karbird/e2e test --grep "project hub|first project|environment|organisation"`

Expected: PASS.

### Task 8: Workspace context switcher and migration of existing pages

**Files:**
- Modify: `apps/web/src/components/workspace/AppShell.tsx`
- Modify: `apps/web/src/components/workspace/Workspace.tsx`
- Modify: `apps/web/src/components/integration/IntegrationGuide.tsx`
- Modify: `apps/web/src/components/tokens/TokensPage.tsx`
- Modify: `apps/web/src/app/app/[projectId]/[environmentId]/**`
- Test: `apps/web/src/components/workspace/AppShell.test.tsx`
- Test: `tests/e2e/specs/navigation.spec.ts`

**Interfaces:**
- Consumes `SessionResponse.organizations` and `ProjectDto.organization_id`.
- Produces a compact accessible `OrganizationProjectEnvironmentSwitcher` that routes only to allowed environments.

- [ ] **Step 1: Write failing switcher tests**

Assert the switcher shows the selected organisation/project/environment, excludes unassigned projects, preserves the destination subpage where valid, and returns to ProjectHub when the selected scope disappears.

- [ ] **Step 2: Run tests to verify failure**

Run: `pnpm --filter @karbird/web test -- AppShell; node scripts/test-with-fixtures.mjs pnpm --filter @karbird/e2e test --grep "switch environment|project switcher"`

Expected: FAIL because navigation has only flat project/environment context.

- [ ] **Step 3: Implement the scope switcher and route compatibility**

Use URL-encoded path values and existing scope route shape. Keep all telemetry and token requests scoped through `scopedPath`; do not put organisation selection into telemetry query parameters.

- [ ] **Step 4: Run focused tests**

Run: `pnpm --filter @karbird/web test; node scripts/test-with-fixtures.mjs pnpm --filter @karbird/e2e test --grep "switch environment|project switcher"`

Expected: PASS.

### Task 9: Production configuration, docs and complete verification

**Files:**
- Modify: `.env.example`
- Modify: `deploy/compose.app.yaml`
- Modify: `scripts/deploy-config.mjs`
- Modify: `.github/workflows/deploy.yml`
- Modify: `docs/operations/deploy.md`
- Modify: `docs/adr/README.md`
- Create: `docs/adr/0021-organisations-and-external-auth.md`
- Test: `scripts/test/deploy-config.test.mjs`
- Test: `tests/integration/api-surface.test.mjs`

**Interfaces:**
- Production API receives `KARBIRD_GOOGLE_CLIENT_ID`, `KARBIRD_GOOGLE_CLIENT_SECRET`, `KARBIRD_GOOGLE_REDIRECT_URL`, `KARBIRD_RESEND_API_KEY`, `KARBIRD_MAIL_FROM` from rendered server `.env` only.

- [ ] **Step 1: Write failing deployment configuration tests**

Assert required OAuth/Resend values render only when configured, are absent from browser build inputs, and missing credential pairs fail with an actionable deployment error.

- [ ] **Step 2: Run test to verify failure**

Run: `node --test scripts/test/deploy-config.test.mjs`

Expected: FAIL because the deployment renderer does not know these keys.

- [ ] **Step 3: Implement deploy wiring and ADR**

Read credential values from GitHub Secrets in the workflow, pass them to `deploy-config.mjs`, and render them with mode 600. Document the exact Google callback and GitHub Secrets names without values. Record architectural choices in ADR-0021.

- [ ] **Step 4: Run full verification**

Run: `just codegen; just check; just test; just test-integration; just test-e2e`

Expected: all checks PASS with `just dev` and `just demo` running.

- [ ] **Step 5: Refresh the code graph**

Run: `graphify update .`

Expected: `graphify-out/graph.html`, `graph.json` and `GRAPH_REPORT.md` updated.
