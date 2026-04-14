ALTER TABLE artifacts
ADD COLUMN status TEXT NOT NULL DEFAULT 'Ready',
ADD COLUMN object_key TEXT;

CREATE TABLE artifact_upload_sessions (
    id UUID PRIMARY KEY,
    artifact_id UUID NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_artifact_upload_sessions_artifact_id
    ON artifact_upload_sessions (artifact_id);
