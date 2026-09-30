-- Invite links to join a project with a role, and onboarding state of accounts.

CREATE TABLE project_invites (
    id            UUID PRIMARY KEY,
    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    role_id       UUID NOT NULL REFERENCES project_roles(id) ON DELETE CASCADE,
    -- SHA-256 of the high-entropy secret; the secret itself is never stored.
    token_hash    BYTEA NOT NULL UNIQUE,
    token_prefix  TEXT NOT NULL,
    created_by    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL,
    max_uses      INTEGER CHECK (max_uses IS NULL OR max_uses BETWEEN 1 AND 1000),
    uses          INTEGER NOT NULL DEFAULT 0 CHECK (uses >= 0),
    revoked_at    TIMESTAMPTZ
);
CREATE INDEX project_invites_project_idx ON project_invites(project_id, created_at DESC);

ALTER TABLE users ADD COLUMN onboarding_completed_at TIMESTAMPTZ;
-- Accounts that existed before onboarding are not sent through it.
UPDATE users SET onboarding_completed_at = now();
