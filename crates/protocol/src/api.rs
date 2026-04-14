use std::collections::BTreeMap;

use domain::{Artifact, Task, TaskLease};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskSubmitRequest {
    pub tenant_id: Uuid,
    pub artifact_id: Uuid,
    pub runtime_pack_id: String,
    pub payload: Option<Value>,
    pub priority: Option<i32>,
    pub rate_limit_key: Option<String>,
    pub timeout_seconds: Option<i32>,
    pub max_attempts: Option<i32>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskResponse {
    pub id: Uuid,
    pub status: domain::TaskStatus,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArtifactRegisterRequest {
    pub tenant_id: Uuid,
    pub digest: String,
    pub runtime_pack_id: String,
    pub entrypoint: String,
    pub size_bytes: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ArtifactResponse {
    pub artifact: Artifact,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PollRequest {
    pub runtime_pack_id: String,
    pub executor_id: Uuid,
    pub lease_duration_sec: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PollResponse {
    pub task: Option<Task>,
    pub lease: Option<TaskLease>,
    pub environment: BTreeMap<String, String>,
    pub payload: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterExecutorRequest {
    pub executor_id: Uuid,
    pub session_id: Uuid,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub executor_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskStatusUpdateRequest {
    pub status: String,
    pub lease_id: Uuid,
    pub executor_id: Uuid,
    pub logs_ref: Option<String>,
    pub result_ref: Option<String>,
    pub last_error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PutRateLimitRequest {
    pub tenant_id: Uuid,
    pub key: String,
    pub max_inflight: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PutSecretRequest {
    pub tenant_id: Uuid,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LiveLogChunkRequest {
    pub lease_id: Uuid,
    pub executor_id: Uuid,
    pub stream: String,
    pub chunk: String,
}
