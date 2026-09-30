-- Project role templates and permission-based memberships.
-- A membership holds exactly one of: a role template (role_id) or an individual permission set.

CREATE TABLE project_roles (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id   TEXT NOT NULL REFERENCES projects(id),
    name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    permissions  TEXT[] NOT NULL DEFAULT '{}',
    builtin      TEXT CHECK (builtin IN ('owner', 'admin', 'operator', 'viewer')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX project_roles_name_idx ON project_roles (project_id, lower(name));
CREATE UNIQUE INDEX project_roles_builtin_idx ON project_roles (project_id, builtin) WHERE builtin IS NOT NULL;

-- Built-in roles of one project; kept in sync with `Builtin::permissions` in controlhub-api.
CREATE FUNCTION controlhub_insert_builtin_roles(p TEXT) RETURNS void LANGUAGE sql AS $$
    INSERT INTO project_roles (project_id, name, permissions, builtin) VALUES
        (p, 'Owner', ARRAY['telemetry.view','tokens.view','tokens.manage','actions.view','actions.propose',
                           'actions.approve','members.view','members.manage','roles.manage','project.manage'], 'owner'),
        (p, 'Admin', ARRAY['telemetry.view','tokens.view','tokens.manage','actions.view','actions.propose',
                           'actions.approve','members.view','members.manage','roles.manage','project.manage'], 'admin'),
        (p, 'Operator', ARRAY['telemetry.view','tokens.view','actions.view','actions.propose','members.view'], 'operator'),
        (p, 'Viewer', ARRAY['telemetry.view','actions.view','members.view'], 'viewer');
$$;

CREATE FUNCTION controlhub_builtin_roles_trigger() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM controlhub_insert_builtin_roles(NEW.id);
    RETURN NEW;
END $$;

CREATE TRIGGER projects_builtin_roles AFTER INSERT ON projects
    FOR EACH ROW EXECUTE FUNCTION controlhub_builtin_roles_trigger();

SELECT controlhub_insert_builtin_roles(id) FROM projects;

ALTER TABLE memberships
    ADD COLUMN role_id UUID REFERENCES project_roles(id),
    ADD COLUMN permissions TEXT[],
    ADD COLUMN based_on UUID REFERENCES project_roles(id) ON DELETE SET NULL;
UPDATE memberships m SET role_id = r.id
    FROM project_roles r WHERE r.project_id = m.project_id AND r.builtin = m.role;
ALTER TABLE memberships DROP COLUMN role;
ALTER TABLE memberships ADD CONSTRAINT memberships_one_grant CHECK ((role_id IS NULL) <> (permissions IS NULL));
CREATE INDEX memberships_project_idx ON memberships (project_id);
