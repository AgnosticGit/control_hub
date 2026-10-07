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

