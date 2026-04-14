use application::ports::repositories::{ExecutorRepository, RepositoryError, TaskRepository};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use protocol::api::{
    HeartbeatRequest, PollRequest, PollResponse, RegisterExecutorRequest, TaskResponse,
    TaskStatusUpdateRequest, TaskSubmitRequest,
};
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub async fn health() -> impl IntoResponse {
    let uptime = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let body = json!({
        "status": "healthy",
        "uptime_seconds": uptime,
    });

    (StatusCode::OK, Json(body))
}

pub struct AppState {
    pub task_repo: Arc<dyn TaskRepository>,
    pub executor_repo: Arc<dyn ExecutorRepository>,
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
            RepositoryError::Conflict(msg) => ApiError::DatabaseError(msg),
        }
    }
}

pub async fn submit_task(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<TaskSubmitRequest>,
) -> impl IntoResponse {
    let task_id = Uuid::new_v4();
    let task = domain::Task {
        id: task_id,
        tenant_id: payload.tenant_id,
        artifact_id: payload.artifact_id,
        runtime_pack_id: payload.runtime_pack_id,
        status: domain::TaskStatus::Queued,
        priority: payload.priority.unwrap_or(0),
        rate_limit_key: payload.rate_limit_key,
        timeout_seconds: payload.timeout_seconds.unwrap_or(300),
        max_attempts: payload.max_attempts.unwrap_or(3),
        current_attempt: 0,
        idempotency_key: payload.idempotency_key,
    };

    match state.task_repo.insert_task(&task).await {
        Ok(_) => {
            let response = TaskResponse {
                id: task.id,
                status: task.status,
            };
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(RepositoryError::Conflict(msg)) => {
            tracing::warn!("Task submission rejected: {}", msg);
            (
                StatusCode::CONFLICT,
                Json(json!({ "error": msg })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Database error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.task_repo.get_task(id).await {
        Ok(Some(task)) => {
            let response = TaskResponse {
                id: task.id,
                status: task.status,
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Database error: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn poll_tasks(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PollRequest>,
) -> impl IntoResponse {
    match state
        .task_repo
        .poll_and_lease(
            &payload.runtime_pack_id,
            payload.executor_id,
            payload.lease_duration_sec,
        )
        .await
    {
        Ok(Some((task, lease))) => {
            let response = PollResponse {
                task: Some(task),
                lease: Some(lease),
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => {
            let response = PollResponse {
                task: None,
                lease: None,
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(RepositoryError::Conflict(msg)) => {
            tracing::warn!("Poll rejected: {}", msg);
            (
                StatusCode::CONFLICT,
                Json(json!({ "error": msg })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Database error during poll: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn register_executor(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RegisterExecutorRequest>,
) -> impl IntoResponse {
    let executor = domain::Executor {
        id: payload.executor_id,
        session_id: payload.session_id,
        capabilities: payload.capabilities,
    };

    match state.executor_repo.upsert_executor(&executor).await {
        Ok(_) => StatusCode::CREATED.into_response(),
        Err(e) => {
            tracing::error!("Database error during executor registration: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn heartbeat_executor(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    match state.executor_repo.record_heartbeat(payload.executor_id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(RepositoryError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Database error during executor heartbeat: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn update_task_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<TaskStatusUpdateRequest>,
) -> impl IntoResponse {
    let status = match payload.status.as_str() {
        "Running" => domain::TaskStatus::Running,
        "Succeeded" => domain::TaskStatus::Succeeded,
        "Failed" => domain::TaskStatus::Failed,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };

    match state
        .task_repo
        .update_task_status(
            id,
            payload.lease_id,
            payload.executor_id,
            status,
            payload.logs_ref,
            payload.result_ref,
            payload.last_error_message,
        )
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(RepositoryError::Conflict(_)) => StatusCode::CONFLICT.into_response(),
        Err(e) => {
            tracing::error!("Database error during status update: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn cancel_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.task_repo.cancel_task(id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(RepositoryError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(RepositoryError::Conflict(_)) => StatusCode::CONFLICT.into_response(),
        Err(e) => {
            tracing::error!("Database error during cancellation: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
