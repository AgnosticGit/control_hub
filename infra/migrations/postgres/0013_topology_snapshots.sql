-- Scope-bound saved maps. A fencing token prevents expired builders publishing late.
CREATE TABLE topology_snapshots (
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    environment_id TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    cache_key TEXT NOT NULL CHECK (length(cache_key) <= 128),
    document JSONB,
    refreshed_at TIMESTAMPTZ,
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    watch_token UUID,
    watch_until TIMESTAMPTZ,
    accessed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, project_id, environment_id, cache_key),
    CHECK (document IS NULL OR octet_length(document::text) <= 16777216)
);
CREATE INDEX topology_snapshots_accessed ON topology_snapshots (accessed_at);
