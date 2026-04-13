use application::ports::repositories::{RepositoryError, TaskRepository};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use domain::{Task, TaskStatus};
use protocol::api::{PollRequest, PollResponse, TaskResponse, TaskSubmitRequest};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub struct AppState {
    pub task_repo: Arc<dyn TaskRepository>,
}

pub enum ApiError {
    DatabaseError(String),
    NotFound,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ApiError::DatabaseError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "Record not found".to_string()),
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

impl From<RepositoryError> for ApiError {
    fn from(err: RepositoryError) -> Self {
        match err {
            RepositoryError::DatabaseError(msg) => ApiError::DatabaseError(msg),
            RepositoryError::NotFound => ApiError::NotFound,
        }
    }
}

pub async fn submit_task(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<TaskSubmitRequest>,
) -> Result<(StatusCode, Json<TaskResponse>), ApiError> {
    let task_id = Uuid::new_v4();
    let task = Task {
        id: task_id,
        tenant_id: payload.tenant_id,
        artifact_id: payload.artifact_id,
        runtime_pack_id: payload.runtime_pack_id,
        status: TaskStatus::Queued,
        priority: payload.priority.unwrap_or(0),
        rate_limit_key: payload.rate_limit_key,
        timeout_seconds: payload.timeout_seconds.unwrap_or(300),
        max_attempts: payload.max_attempts.unwrap_or(3),
        current_attempt: 0,
        idempotency_key: payload.idempotency_key,
    };

    state.task_repo.insert_task(&task).await?;

    Ok((StatusCode::CREATED, Json(TaskResponse { task })))
}

pub async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<TaskResponse>, ApiError> {
    let task = state.task_repo.get_task(id).await?;
    Ok(Json(TaskResponse { task }))
}

pub async fn poll_tasks(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PollRequest>,
) -> Result<Json<PollResponse>, ApiError> {
    let result = state
        .task_repo
        .poll_and_lease(
            &payload.runtime_pack_id,
            payload.executor_id,
            payload.lease_duration_sec,
        )
        .await?;

    match result {
        Some((task, lease)) => Ok(Json(PollResponse {
            task: Some(task),
            lease: Some(lease),
        })),
        None => Ok(Json(PollResponse {
            task: None,
            lease: None,
        })),
    }
}
