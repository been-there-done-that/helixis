CREATE TABLE tenants (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE runtime_packs (
    id TEXT PRIMARY KEY,
    language TEXT NOT NULL,
    language_version TEXT NOT NULL,
    sandbox_kind TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE artifacts (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    digest TEXT NOT NULL UNIQUE,
    runtime_pack_id TEXT NOT NULL REFERENCES runtime_packs(id),
    entrypoint TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE tasks (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    artifact_id UUID NOT NULL REFERENCES artifacts(id),
    runtime_pack_id TEXT NOT NULL REFERENCES runtime_packs(id),
    status TEXT NOT NULL,
    priority INT NOT NULL DEFAULT 0,
    rate_limit_key TEXT,
    cancel_requested_at TIMESTAMPTZ,
    payload_ref TEXT,
    timeout_seconds INT NOT NULL,
    max_attempts INT NOT NULL DEFAULT 3,
    current_attempt INT NOT NULL DEFAULT 0,
    idempotency_key TEXT,
    scheduled_at TIMESTAMPTZ,
    not_before TIMESTAMPTZ,
    result_ref TEXT,
    payload_size_bytes BIGINT,
    result_size_bytes BIGINT,
    log_size_bytes BIGINT,
    logs_truncated BOOLEAN NOT NULL DEFAULT FALSE,
    last_error_code TEXT,
    last_error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for queue polling
CREATE INDEX idx_tasks_queue ON tasks(status, runtime_pack_id, not_before, priority, tenant_id);
CREATE UNIQUE INDEX idx_tasks_idempotency ON tasks(tenant_id, idempotency_key) WHERE idempotency_key IS NOT NULL;

CREATE TABLE executors (
    id UUID PRIMARY KEY,
    session_id UUID NOT NULL,
    capabilities_json JSONB NOT NULL,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE task_leases (
    id UUID PRIMARY KEY,
    task_id UUID NOT NULL UNIQUE REFERENCES tasks(id) ON DELETE CASCADE,
    executor_id UUID NOT NULL REFERENCES executors(id),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE task_attempts (
    id UUID PRIMARY KEY,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    executor_id UUID NOT NULL REFERENCES executors(id),
    lease_id UUID NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ,
    exit_reason TEXT,
    logs_ref TEXT,
    result_ref TEXT
);
