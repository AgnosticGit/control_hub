-- ControlHub operational metadata. Telemetry lives in ClickHouse.

CREATE TABLE tenants (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE projects (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL REFERENCES tenants(id),
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, name)
);

CREATE TABLE environments (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL REFERENCES tenants(id),
    project_id  TEXT NOT NULL REFERENCES projects(id),
    name        TEXT NOT NULL,
    -- Demo environments are always labelled as such in UI and API.
    is_demo     BOOLEAN NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);

CREATE TABLE users (
    id             UUID PRIMARY KEY,
    email          TEXT NOT NULL UNIQUE,
    display_name   TEXT NOT NULL,
    -- Argon2id PHC string.
    password_hash  TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    disabled_at    TIMESTAMPTZ
);

CREATE TABLE memberships (
    user_id     UUID NOT NULL REFERENCES users(id),
    project_id  TEXT NOT NULL REFERENCES projects(id),
    role        TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'operator', 'viewer')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, project_id)
);

CREATE TABLE sessions (
    id               UUID PRIMARY KEY,
    user_id          UUID NOT NULL REFERENCES users(id),
    -- SHA-256 of the high-entropy session token; the token itself is never stored.
    token_hash       BYTEA NOT NULL UNIQUE,
    csrf_token_hash  BYTEA NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at       TIMESTAMPTZ NOT NULL,
    revoked_at       TIMESTAMPTZ,
    last_seen_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sessions_user_idx ON sessions(user_id);

CREATE TABLE ingest_tokens (
    id              UUID PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id),
    project_id      TEXT NOT NULL REFERENCES projects(id),
    environment_id  TEXT NOT NULL REFERENCES environments(id),
    name            TEXT NOT NULL,
    -- Non-secret prefix for identification in UI/logs.
    token_prefix    TEXT NOT NULL,
    token_hash      BYTEA NOT NULL UNIQUE,
    signals         TEXT[] NOT NULL DEFAULT ARRAY['traces','logs','events'],
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ,
    revoked_at      TIMESTAMPTZ,
    created_by      UUID REFERENCES users(id)
);

-- Broker consumer progress. Advanced only after the batch is stored in
-- ClickHouse or recorded in processing_errors (dead-letter).
CREATE TABLE consumer_offsets (
    consumer               TEXT NOT NULL,
    topic                  TEXT NOT NULL,
    partition              INTEGER NOT NULL,
    next_offset            BIGINT NOT NULL,
    high_watermark         BIGINT NOT NULL DEFAULT 0,
    last_received_time_ns  BIGINT NOT NULL DEFAULT 0,
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (consumer, topic, partition)
);

-- Dead-letter records: nothing is dropped silently.
CREATE TABLE processing_errors (
    id              UUID PRIMARY KEY,
    consumer        TEXT NOT NULL,
    topic           TEXT NOT NULL,
    partition       INTEGER NOT NULL,
    broker_offset   BIGINT NOT NULL,
    batch_id        TEXT,
    tenant_id       TEXT,
    error_kind      TEXT NOT NULL,
    error_message   TEXT NOT NULL,
    -- Raw record (bounded size) so the batch can be replayed after a fix.
    payload         BYTEA,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at     TIMESTAMPTZ,
    UNIQUE (consumer, topic, partition, broker_offset)
);

-- Operational audit. A regular table: NOT tamper-proof.
CREATE TABLE audit_log (
    id           UUID PRIMARY KEY,
    occurred_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    tenant_id    TEXT,
    project_id   TEXT,
    actor_id     TEXT NOT NULL,
    actor_kind   TEXT NOT NULL CHECK (actor_kind IN ('user', 'system', 'token')),
    action       TEXT NOT NULL,
    target       TEXT,
    reason       TEXT,
    before       JSONB,
    after        JSONB,
    outcome      TEXT NOT NULL,
    request_id   TEXT
);
CREATE INDEX audit_log_scope_idx ON audit_log(tenant_id, project_id, occurred_at DESC);
