-- Cold archive catalog (ADR-0015). Segment and dictionary bytes live in S3-compatible object
-- storage; these tables say which object holds which scope, service and time window.

-- One segment: one service's logs for one archive window.
CREATE TABLE archive_segments (
    tenant_id        TEXT NOT NULL,
    project_id       TEXT NOT NULL,
    environment_id   TEXT NOT NULL,
    signal           TEXT NOT NULL CHECK (signal IN ('logs')),
    service_id       TEXT NOT NULL,
    window_start_ns  BIGINT NOT NULL,
    window_end_ns    BIGINT NOT NULL CHECK (window_end_ns > window_start_ns),
    min_time_ns      BIGINT NOT NULL,
    max_time_ns      BIGINT NOT NULL,
    rows             BIGINT NOT NULL CHECK (rows >= 0),
    source_bytes     BIGINT NOT NULL,
    stored_bytes     BIGINT NOT NULL,
    templates        INTEGER NOT NULL,
    -- 0: compressed without a dictionary.
    dict_version     INTEGER NOT NULL DEFAULT 0,
    object_key       TEXT NOT NULL,
    sha256           TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, project_id, environment_id, signal, service_id, window_start_ns)
);
CREATE INDEX archive_segments_scope_time_idx
    ON archive_segments (tenant_id, project_id, environment_id, signal, max_time_ns);
CREATE INDEX archive_segments_window_end_idx ON archive_segments (window_end_ns);

-- Per-service zstd dictionaries; a segment names the version it was compressed with.
CREATE TABLE archive_dictionaries (
    tenant_id        TEXT NOT NULL,
    project_id       TEXT NOT NULL,
    environment_id   TEXT NOT NULL,
    signal           TEXT NOT NULL CHECK (signal IN ('logs')),
    service_id       TEXT NOT NULL,
    version          INTEGER NOT NULL CHECK (version > 0),
    object_key       TEXT NOT NULL,
    entries          INTEGER NOT NULL,
    bytes            INTEGER NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, project_id, environment_id, signal, service_id, version)
);

-- Archiver progress over all tenants: a window is archived once (and re-checked once later for
-- late data) or recorded as lost when it expired from ClickHouse before it could be archived.
CREATE TABLE archive_windows (
    signal           TEXT NOT NULL CHECK (signal IN ('logs')),
    window_start_ns  BIGINT NOT NULL,
    window_end_ns    BIGINT NOT NULL CHECK (window_end_ns > window_start_ns),
    status           TEXT NOT NULL CHECK (status IN ('archived', 'lost')),
    rows             BIGINT NOT NULL DEFAULT 0,
    segments         INTEGER NOT NULL DEFAULT 0,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    rechecked_at     TIMESTAMPTZ,
    PRIMARY KEY (signal, window_start_ns)
);
