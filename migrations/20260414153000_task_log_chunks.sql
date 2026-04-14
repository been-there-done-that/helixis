CREATE TABLE task_log_chunks (
    id UUID PRIMARY KEY,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    lease_id UUID NOT NULL,
    executor_id UUID NOT NULL REFERENCES executors(id),
    stream TEXT NOT NULL,
    chunk TEXT NOT NULL,
    chunk_index BIGSERIAL NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_task_log_chunks_task
    ON task_log_chunks(task_id, chunk_index ASC);
