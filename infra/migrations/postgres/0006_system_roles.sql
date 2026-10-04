-- karbird administrators: installation-wide role templates and grants, with the same model
-- as project memberships (a grant holds a role template or an individual permission set).

CREATE TABLE system_roles (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    permissions  TEXT[] NOT NULL DEFAULT '{}',
    builtin      TEXT UNIQUE CHECK (builtin IN ('super_admin', 'admin', 'auditor')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX system_roles_name_idx ON system_roles (lower(name));

-- Kept in sync with `SystemBuiltin::permissions` in karbird-api.
INSERT INTO system_roles (name, permissions, builtin) VALUES
    ('Super Admin', ARRAY['system.projects.view','system.projects.manage','system.users.view','system.users.manage',
                          'system.team.view','system.team.manage','system.roles.manage','system.audit.view'], 'super_admin'),
    ('Administrator', ARRAY['system.projects.view','system.projects.manage','system.users.view','system.users.manage',
                            'system.team.view','system.team.manage','system.roles.manage','system.audit.view'], 'admin'),
    ('Auditor', ARRAY['system.projects.view','system.users.view','system.team.view','system.audit.view'], 'auditor');

CREATE TABLE system_memberships (
    user_id      UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    role_id      UUID REFERENCES system_roles(id),
    permissions  TEXT[],
    based_on     UUID REFERENCES system_roles(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT system_memberships_one_grant CHECK ((role_id IS NULL) <> (permissions IS NULL))
);

-- Deletes a project and every PostgreSQL row that belongs to it; its telemetry in ClickHouse
-- expires with the table TTL. Runs with the owner's rights so the API role needs no DELETE on
-- these tables: it may only call this function (granted in bootstrap).
CREATE FUNCTION karbird_delete_project(p TEXT) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    t TEXT;
BEGIN
    SELECT tenant_id INTO t FROM projects WHERE id = p FOR UPDATE;
    IF NOT FOUND THEN
        RETURN false;
    END IF;
    DELETE FROM action_plans WHERE project_id = p;
    DELETE FROM api_tokens WHERE project_id = p;
    DELETE FROM ingest_tokens WHERE project_id = p;
    DELETE FROM project_invites WHERE project_id = p;
    DELETE FROM memberships WHERE project_id = p;
    DELETE FROM project_roles WHERE project_id = p;
    DELETE FROM environments WHERE project_id = p;
    DELETE FROM projects WHERE id = p;
    IF NOT EXISTS (SELECT 1 FROM projects WHERE tenant_id = t) THEN
        DELETE FROM tenants WHERE id = t;
    END IF;
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION karbird_delete_project(TEXT) FROM PUBLIC;
