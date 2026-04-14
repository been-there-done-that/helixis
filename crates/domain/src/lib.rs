use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Queued,
    Scheduled,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    DeadLetter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub artifact_id: Uuid,
    pub runtime_pack_id: String,
    pub status: TaskStatus,
    pub priority: i32,
    pub rate_limit_key: Option<String>,
    pub payload_ref: Option<String>,
    pub timeout_seconds: i32,
    pub max_attempts: i32,
    pub current_attempt: i32,
    pub idempotency_key: Option<String>,
    pub payload_size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePack {
    pub id: String,
    pub language: String,
    pub language_version: String,
    pub sandbox_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtifactStatus {
    PendingUpload,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub digest: String,
    pub runtime_pack_id: String,
    pub entrypoint: String,
    pub size_bytes: i64,
    pub status: ArtifactStatus,
    pub object_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArtifactUploadStatus {
    Pending,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactUploadSession {
    pub id: Uuid,
    pub artifact_id: Uuid,
    pub status: ArtifactUploadStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutputs {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub logs_ref: Option<String>,
    pub result_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLease {
    pub id: Uuid,
    pub task_id: Uuid,
    pub executor_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Executor {
    pub id: Uuid,
    pub session_id: Uuid,
    pub capabilities: Vec<String>,
}
