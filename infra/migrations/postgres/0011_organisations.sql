-- Organisations reuse the existing tenant identifier. This keeps every
-- telemetry scope and external token valid while separating membership in an
-- organisation from access to one of its projects.

CREATE TABLE IF NOT EXISTS organisations (
    id          TEXT PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS organisation_memberships (
    user_id          UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    organisation_id  TEXT NOT NULL REFERENCES organisations(id) ON DELETE CASCADE,
    role             TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, organisation_id)
);
CREATE INDEX IF NOT EXISTS organisation_memberships_organisation_idx
    ON organisation_memberships (organisation_id, user_id);

-- This table deliberately duplicates the effective project grant from the
-- legacy `memberships` table. The legacy table remains the source of truth
-- until the authorization migration can switch reads atomically.
CREATE TABLE IF NOT EXISTS project_memberships (
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    role_id      UUID REFERENCES project_roles(id) ON DELETE SET NULL,
    permissions  TEXT[],
    based_on     UUID REFERENCES project_roles(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, project_id),
    CHECK ((role_id IS NULL) <> (permissions IS NULL))
);
CREATE INDEX IF NOT EXISTS project_memberships_project_idx
    ON project_memberships (project_id, user_id);

CREATE TABLE IF NOT EXISTS oauth_identities (
    provider    TEXT NOT NULL CHECK (provider IN ('google')),
    subject     TEXT NOT NULL,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    email       TEXT NOT NULL,
    linked_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject),
    UNIQUE (provider, user_id)
);

CREATE TABLE IF NOT EXISTS organisation_invites (
    id                  UUID PRIMARY KEY,
    organisation_id     TEXT NOT NULL REFERENCES organisations(id) ON DELETE CASCADE,
    email               TEXT NOT NULL,
    organisation_role   TEXT NOT NULL CHECK (organisation_role IN ('admin', 'member')),
    project_assignments JSONB NOT NULL DEFAULT '[]'::jsonb,
    token_hash          BYTEA NOT NULL UNIQUE,
    token_prefix        TEXT NOT NULL,
    created_by          UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at          TIMESTAMPTZ NOT NULL,
    revoked_at          TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS organisation_invites_organisation_idx
    ON organisation_invites (organisation_id, created_at DESC);

-- Existing tenants become organisations without changing their IDs.
INSERT INTO organisations (id, name, created_at)
SELECT id, name, created_at FROM tenants
ON CONFLICT (id) DO NOTHING;

-- Every legacy project member becomes both an organisation member and a
-- project assignment. Owners retain their owner role at organisation scope;
-- other project roles receive the least-privileged organisation role.
INSERT INTO organisation_memberships (user_id, organisation_id, role, created_at)
SELECT m.user_id,
       p.tenant_id,
       CASE WHEN r.builtin = 'owner' THEN 'owner' ELSE 'member' END,
       m.created_at
FROM memberships m
JOIN projects p ON p.id = m.project_id
LEFT JOIN project_roles r ON r.id = m.role_id
ON CONFLICT (user_id, organisation_id) DO UPDATE
SET role = CASE
    WHEN organisation_memberships.role = 'owner' OR EXCLUDED.role = 'owner' THEN 'owner'
    ELSE organisation_memberships.role
END;

INSERT INTO project_memberships (user_id, project_id, role_id, permissions, based_on, created_at)
SELECT user_id, project_id, role_id, permissions, based_on, created_at
FROM memberships
ON CONFLICT (user_id, project_id) DO NOTHING;

-- Legacy role-management routes continue to write `memberships` during the
-- transition. Keep the new authorization records synchronized until those
-- routes move to the new tables in a later migration.
CREATE OR REPLACE FUNCTION karbird_sync_project_membership() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    source memberships%ROWTYPE;
    organisation_role TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        DELETE FROM project_memberships WHERE user_id = OLD.user_id AND project_id = OLD.project_id;
        RETURN OLD;
    END IF;

    source := NEW;
    SELECT CASE WHEN r.builtin = 'owner' THEN 'owner' ELSE 'member' END
    INTO organisation_role
    FROM projects p LEFT JOIN project_roles r ON r.id = source.role_id
    WHERE p.id = source.project_id;

    INSERT INTO organisation_memberships (user_id, organisation_id, role, created_at)
    SELECT source.user_id, p.tenant_id, COALESCE(organisation_role, 'member'), source.created_at
    FROM projects p WHERE p.id = source.project_id
    ON CONFLICT (user_id, organisation_id) DO UPDATE
    SET role = CASE
        WHEN organisation_memberships.role = 'owner' OR EXCLUDED.role = 'owner' THEN 'owner'
        ELSE organisation_memberships.role
    END;

    INSERT INTO project_memberships (user_id, project_id, role_id, permissions, based_on, created_at)
    VALUES (source.user_id, source.project_id, source.role_id, source.permissions, source.based_on, source.created_at)
    ON CONFLICT (user_id, project_id) DO UPDATE
    SET role_id = EXCLUDED.role_id, permissions = EXCLUDED.permissions, based_on = EXCLUDED.based_on;
    RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS memberships_organisation_sync ON memberships;
CREATE TRIGGER memberships_organisation_sync
    AFTER INSERT OR UPDATE OR DELETE ON memberships
    FOR EACH ROW EXECUTE FUNCTION karbird_sync_project_membership();

-- New tenants must also be visible as organisations. The trigger is safe on
-- retries and lets existing scope bootstrap continue to use `tenants`.
CREATE OR REPLACE FUNCTION karbird_insert_organisation_for_tenant() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO organisations (id, name, created_at)
    VALUES (NEW.id, NEW.name, NEW.created_at)
    ON CONFLICT (id) DO NOTHING;
    RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS tenants_organisation_trigger ON tenants;
CREATE TRIGGER tenants_organisation_trigger
    AFTER INSERT ON tenants
    FOR EACH ROW EXECUTE FUNCTION karbird_insert_organisation_for_tenant();
