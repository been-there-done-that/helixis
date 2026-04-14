CREATE TABLE rate_limits (
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    rate_limit_key TEXT NOT NULL,
    max_inflight INT NOT NULL CHECK (max_inflight > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, rate_limit_key)
);

CREATE TABLE secrets (
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    secret_key TEXT NOT NULL,
    encrypted_value TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, secret_key)
);

CREATE INDEX idx_tasks_active_rate_limits
    ON tasks(tenant_id, rate_limit_key, status)
    WHERE rate_limit_key IS NOT NULL
      AND status IN ('Scheduled', 'Running');
