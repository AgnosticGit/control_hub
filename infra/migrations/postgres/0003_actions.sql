-- Controlled actions (ADR-0012). Plans are proposed and approved through the API and executed
-- only by the separate executor, which claims approved plans with FOR UPDATE SKIP LOCKED.
-- Every state change is written in the same transaction as its audit_log entry.

CREATE TABLE action_plans (
    id               UUID PRIMARY KEY,
    tenant_id        TEXT NOT NULL REFERENCES tenants(id),
    project_id       TEXT NOT NULL REFERENCES projects(id),
    environment_id   TEXT NOT NULL REFERENCES environments(id),
    action_type      TEXT NOT NULL,
    target           TEXT NOT NULL,
    params           JSONB NOT NULL,
    reason           TEXT NOT NULL,
    state            TEXT NOT NULL CHECK (state IN ('proposed', 'approved', 'rejected', 'cancelled', 'expired',
                                                    'running', 'succeeded', 'failed', 'verification_failed')),
    policy           JSONB NOT NULL,
    created_by       UUID NOT NULL REFERENCES users(id),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Deadline for the next step: decision while proposed, execution start while approved.
    expires_at       TIMESTAMPTZ NOT NULL,
    decided_by       UUID REFERENCES users(id),
    decided_at       TIMESTAMPTZ,
    decision_comment TEXT,
    executor_id      TEXT,
    lease_until      TIMESTAMPTZ,
    started_at       TIMESTAMPTZ,
    finished_at      TIMESTAMPTZ,
    result           JSONB,
    verification     JSONB,
    -- Approver must differ from the proposer (four-eyes).
    CHECK (decided_by IS NULL OR state IN ('rejected', 'cancelled', 'expired') OR decided_by <> created_by)
);
CREATE INDEX action_plans_scope_idx ON action_plans(tenant_id, project_id, environment_id, created_at DESC);
CREATE INDEX action_plans_open_idx ON action_plans(state) WHERE state IN ('proposed', 'approved', 'running');

-- Audit entries can reference the plan they belong to.
CREATE INDEX audit_log_target_idx ON audit_log(target);
