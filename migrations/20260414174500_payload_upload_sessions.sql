CREATE TABLE payloads (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    digest TEXT NOT NULL UNIQUE,
    size_bytes BIGINT NOT NULL,
    status TEXT NOT NULL,
    object_key TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE payload_upload_sessions (
    id UUID PRIMARY KEY,
    payload_id UUID NOT NULL REFERENCES payloads(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_payload_upload_sessions_payload_id
    ON payload_upload_sessions (payload_id);
