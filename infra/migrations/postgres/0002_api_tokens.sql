-- Read-only API tokens for automation (CLI `context`, MCP server).
-- Scoped to exactly one environment, always expire, stored only as SHA-256 hashes.
-- Effective access is the intersection of the token scope and the creator's current membership.

CREATE TABLE api_tokens (
    id              UUID PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id),
    project_id      TEXT NOT NULL REFERENCES projects(id),
    environment_id  TEXT NOT NULL REFERENCES environments(id),
    name            TEXT NOT NULL,
    token_prefix    TEXT NOT NULL,
    token_hash      BYTEA NOT NULL UNIQUE,
    created_by      UUID NOT NULL REFERENCES users(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL,
    revoked_at      TIMESTAMPTZ,
    last_used_at    TIMESTAMPTZ
);
CREATE INDEX api_tokens_scope_idx ON api_tokens(tenant_id, project_id, environment_id);
CREATE INDEX ingest_tokens_scope_idx ON ingest_tokens(tenant_id, project_id, environment_id);
