-- A Control installation owns one explicitly confirmed Docker container. Credentials are hashes.
CREATE TABLE control_installations (
    id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    environment_id TEXT NOT NULL REFERENCES environments(id),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    enrollment_hash BYTEA UNIQUE,
    enrollment_expires_at TIMESTAMPTZ,
    credential_hash BYTEA UNIQUE,
    container_id TEXT,
    container_name TEXT,
    reported_container_id TEXT,
    reported_container_name TEXT,
    reported_state TEXT,
    confirmed_at TIMESTAMPTZ,
    last_seen_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (container_id IS NULL OR length(container_id) BETWEEN 12 AND 128)
);
CREATE INDEX control_installations_scope_idx
    ON control_installations(tenant_id, project_id, environment_id, created_at DESC);

CREATE TABLE control_commands (
    id UUID PRIMARY KEY,
    plan_id UUID NOT NULL UNIQUE REFERENCES action_plans(id) ON DELETE CASCADE,
    installation_id UUID NOT NULL REFERENCES control_installations(id) ON DELETE CASCADE,
    container_id TEXT NOT NULL,
    verb TEXT NOT NULL CHECK (verb IN ('start', 'stop', 'restart')),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'delivered', 'completed', 'expired')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    delivered_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    result JSONB
);
CREATE INDEX control_commands_poll_idx ON control_commands(installation_id, created_at)
    WHERE state IN ('pending', 'delivered');

-- Project removal is mediated by this function. Remove Control rows before environments.
CREATE OR REPLACE FUNCTION karbird_delete_project(p TEXT) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE t TEXT;
BEGIN
    SELECT tenant_id INTO t FROM projects WHERE id = p FOR UPDATE;
    IF NOT FOUND THEN RETURN false; END IF;
    DELETE FROM action_plans WHERE project_id = p;
    DELETE FROM control_installations WHERE project_id = p;
    DELETE FROM api_tokens WHERE project_id = p;
    DELETE FROM ingest_tokens WHERE project_id = p;
    DELETE FROM project_invites WHERE project_id = p;
    DELETE FROM memberships WHERE project_id = p;
    DELETE FROM project_roles WHERE project_id = p;
    DELETE FROM environments WHERE project_id = p;
    DELETE FROM projects WHERE id = p;
    IF NOT EXISTS (SELECT 1 FROM projects WHERE tenant_id = t) THEN DELETE FROM tenants WHERE id = t; END IF;
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION karbird_delete_project(TEXT) FROM PUBLIC;
