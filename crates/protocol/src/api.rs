use domain::{Task, TaskLease};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskSubmitRequest {
    pub tenant_id: Uuid,
    pub artifact_id: Uuid,
    pub runtime_pack_id: String,
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
pub struct PollRequest {
    pub runtime_pack_id: String,
    pub executor_id: Uuid,
    pub lease_duration_sec: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PollResponse {
    pub task: Option<Task>,
    pub lease: Option<TaskLease>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskStatusUpdateRequest {
    pub status: String,
    pub lease_id: Uuid,
    pub executor_id: Uuid,
}
