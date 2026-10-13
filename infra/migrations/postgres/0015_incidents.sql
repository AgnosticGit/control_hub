-- Service incident rules and durable incident history. Evaluation is scoped and idempotent
-- per closed minute; one open incident per rule is enforced by a partial unique index.

CREATE TABLE incident_rules (
    id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    environment_id TEXT NOT NULL REFERENCES environments(id),
    service_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('error_rate', 'p95_latency')),
    threshold DOUBLE PRECISION NOT NULL CHECK (
        (kind = 'error_rate' AND threshold > 0 AND threshold <= 100) OR
        (kind = 'p95_latency' AND threshold > 0 AND threshold <= 60000)
    ),
    min_requests INTEGER NOT NULL CHECK (min_requests BETWEEN 1 AND 100000),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    last_window_end_ns BIGINT NOT NULL DEFAULT 0,
    bad_streak INTEGER NOT NULL DEFAULT 0 CHECK (bad_streak BETWEEN 0 AND 2),
    good_streak INTEGER NOT NULL DEFAULT 0 CHECK (good_streak BETWEEN 0 AND 2),
    observation_state TEXT NOT NULL DEFAULT 'unknown' CHECK (observation_state IN ('unknown', 'healthy', 'breaching')),
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, project_id, environment_id, service_id, kind)
);
CREATE INDEX incident_rules_due_idx ON incident_rules (enabled, last_window_end_ns) WHERE enabled;

CREATE TABLE incidents (
    id UUID PRIMARY KEY,
    rule_id UUID NOT NULL REFERENCES incident_rules(id),
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    environment_id TEXT NOT NULL REFERENCES environments(id),
    service_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('error_rate', 'p95_latency')),
    threshold DOUBLE PRECISION NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('open', 'recovered', 'closed')),
    opened_window_end_ns BIGINT NOT NULL,
    last_window_end_ns BIGINT NOT NULL,
    requests BIGINT NOT NULL,
    errors BIGINT NOT NULL,
    p95_ms DOUBLE PRECISION,
    acknowledged_by UUID REFERENCES users(id),
    acknowledged_at TIMESTAMPTZ,
    recovered_at TIMESTAMPTZ,
    closed_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX incidents_one_open_per_rule ON incidents(rule_id) WHERE state = 'open';
CREATE INDEX incidents_scope_recent_idx ON incidents(tenant_id, project_id, environment_id, created_at DESC);

-- Existing and newly created built-in roles receive the incident management permission.
-- Keep the full role template here so the project trigger uses it for future projects.
CREATE OR REPLACE FUNCTION karbird_insert_builtin_roles(p TEXT) RETURNS void LANGUAGE sql AS $$
    INSERT INTO project_roles (project_id, name, permissions, builtin) VALUES
        (p, 'Owner', ARRAY['telemetry.view','incidents.manage','tokens.view','tokens.manage','actions.view','actions.propose',
                           'actions.approve','members.view','members.manage','roles.manage','project.manage'], 'owner'),
        (p, 'Admin', ARRAY['telemetry.view','incidents.manage','tokens.view','tokens.manage','actions.view','actions.propose',
                           'actions.approve','members.view','members.manage','roles.manage','project.manage'], 'admin'),
        (p, 'Operator', ARRAY['telemetry.view','incidents.manage','tokens.view','actions.view','actions.propose','members.view'], 'operator'),
        (p, 'Viewer', ARRAY['telemetry.view','actions.view','members.view'], 'viewer');
$$;
UPDATE project_roles SET permissions = array_append(permissions, 'incidents.manage')
WHERE builtin IN ('owner', 'admin', 'operator') AND NOT ('incidents.manage' = ANY(permissions));
