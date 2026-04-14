use application::ports::repositories::{
    ArtifactRepository, ExecutorRepository, RateLimitRepository, RepositoryError, SecretRepository,
    TaskRepository,
};
use axum::{
    Json,
    body::{Body, Bytes, to_bytes},
    extract::{Path, State},
    http::StatusCode,
    http::header::{CONTENT_TYPE, HeaderValue},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use object_store::{ObjectStore, path::Path as StorePath};
use protocol::api::{
    ArtifactResponse, ArtifactUploadCreateRequest, ArtifactUploadSessionResponse, HeartbeatRequest,
    LiveLogChunkRequest, PollRequest, PollResponse, PutRateLimitRequest, PutSecretRequest,
    RegisterExecutorRequest, TaskResponse, TaskStatusUpdateRequest, TaskSubmitRequest,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::{live_logs::LiveLogHub, storage};

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
    pub artifact_repo: Arc<dyn ArtifactRepository>,
    pub executor_repo: Arc<dyn ExecutorRepository>,
    pub rate_limit_repo: Arc<dyn RateLimitRepository>,
    pub secret_repo: Arc<dyn SecretRepository>,
    pub output_store: Arc<dyn ObjectStore>,
    pub artifact_store: Arc<dyn ObjectStore>,
    pub log_hub: Arc<LiveLogHub>,
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
    let artifact = match state.artifact_repo.get_artifact(payload.artifact_id).await {
        Ok(Some(artifact)) => artifact,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("Failed to load artifact during task submission: {:?}", err);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if artifact.tenant_id != payload.tenant_id {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "artifact does not belong to tenant" })),
        )
            .into_response();
    }

    if artifact.runtime_pack_id != payload.runtime_pack_id {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "artifact runtime does not match task runtime pack" })),
        )
            .into_response();
    }

    if artifact.status != domain::ArtifactStatus::Ready || artifact.object_key.is_none() {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "artifact is not ready for execution" })),
        )
            .into_response();
    }

    let task_id = Uuid::new_v4();
    let (payload_ref, payload_size_bytes) = match payload.payload {
        Some(payload_json) => {
            let bytes = match serde_json::to_vec(&payload_json) {
                Ok(bytes) => bytes,
                Err(err) => {
                    tracing::error!("Failed to serialize task payload: {}", err);
                    return StatusCode::BAD_REQUEST.into_response();
                }
            };
            let key = format!("tasks/{task_id}/payload.json");
            if let Err(err) = storage::put_bytes(&state.output_store, &key, bytes.clone()).await {
                tracing::error!("Failed to upload task payload: {}", err);
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            (Some(key), Some(bytes.len() as i64))
        }
        None => (None, None),
    };
    let task = domain::Task {
        id: task_id,
        tenant_id: payload.tenant_id,
        artifact_id: payload.artifact_id,
        runtime_pack_id: payload.runtime_pack_id,
        status: domain::TaskStatus::Queued,
        priority: payload.priority.unwrap_or(0),
        rate_limit_key: payload.rate_limit_key,
        payload_ref,
        timeout_seconds: payload.timeout_seconds.unwrap_or(300),
        max_attempts: payload.max_attempts.unwrap_or(3),
        current_attempt: 0,
        idempotency_key: payload.idempotency_key,
        payload_size_bytes,
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
            (StatusCode::CONFLICT, Json(json!({ "error": msg }))).into_response()
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

pub async fn get_task_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    fetch_task_blob(state, id, BlobKind::Logs).await
}

pub async fn stream_task_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let historical = state
        .task_repo
        .list_task_log_chunks(id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|chunk| Ok(Event::default().data(chunk)));
    let receiver = state.log_hub.subscribe(id).await;
    let live = BroadcastStream::new(receiver).filter_map(|message| match message {
        Ok(chunk) => Some(Ok(Event::default().data(chunk))),
        Err(_) => None,
    });
    let stream = tokio_stream::iter(historical).chain(live);

    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(5)))
}

pub async fn append_task_log(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(payload): Json<LiveLogChunkRequest>,
) -> impl IntoResponse {
    match state
        .task_repo
        .append_task_log_chunk(
            id,
            payload.lease_id,
            payload.executor_id,
            &payload.stream,
            &payload.chunk,
        )
        .await
    {
        Ok(()) => {
            let chunk = format!("[{}] {}", payload.stream, payload.chunk);
            state.log_hub.publish(id, chunk).await;
            StatusCode::NO_CONTENT
        }
        Err(RepositoryError::Conflict(_)) => StatusCode::CONFLICT,
        Err(err) => {
            tracing::error!("Failed to append task log chunk: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

pub async fn get_task_result(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    fetch_task_blob(state, id, BlobKind::Result).await
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
            let artifact = match state.artifact_repo.get_artifact(task.artifact_id).await {
                Ok(Some(artifact)) => artifact,
                Ok(None) => {
                    tracing::error!(
                        "Task {} references missing artifact {}",
                        task.id,
                        task.artifact_id
                    );
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                Err(err) => {
                    tracing::error!("Failed to load artifact during poll: {:?}", err);
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };
            let payload =
                match load_task_payload(&state.output_store, task.payload_ref.as_deref()).await {
                    Ok(payload) => payload,
                    Err(err) => {
                        tracing::error!("Failed to load task payload during poll: {}", err);
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                };
            let environment = match state.secret_repo.get_tenant_secrets(task.tenant_id).await {
                Ok(environment) => environment,
                Err(err) => {
                    tracing::error!("Failed to load task secrets during poll: {:?}", err);
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };
            let response = PollResponse {
                task: Some(task),
                lease: Some(lease),
                artifact: Some(artifact),
                environment,
                payload,
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => {
            let response = PollResponse {
                task: None,
                lease: None,
                artifact: None,
                environment: BTreeMap::new(),
                payload: None,
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(RepositoryError::Conflict(msg)) => {
            tracing::warn!("Poll rejected: {}", msg);
            (StatusCode::CONFLICT, Json(json!({ "error": msg }))).into_response()
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

pub async fn create_artifact_upload(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ArtifactUploadCreateRequest>,
) -> impl IntoResponse {
    let artifact = domain::Artifact {
        id: Uuid::new_v4(),
        tenant_id: payload.tenant_id,
        digest: payload.digest,
        runtime_pack_id: payload.runtime_pack_id,
        entrypoint: payload.entrypoint,
        size_bytes: payload.size_bytes,
        status: domain::ArtifactStatus::PendingUpload,
        object_key: None,
    };

    match state.artifact_repo.create_artifact_upload(&artifact).await {
        Ok(upload_session) => (
            StatusCode::CREATED,
            Json(ArtifactUploadSessionResponse {
                artifact,
                upload_session,
            }),
        )
            .into_response(),
        Err(RepositoryError::Conflict(msg)) => {
            (StatusCode::CONFLICT, Json(json!({ "error": msg }))).into_response()
        }
        Err(err) => {
            tracing::error!("Database error during artifact registration: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn get_artifact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.artifact_repo.get_artifact(id).await {
        Ok(Some(artifact)) => (StatusCode::OK, Json(ArtifactResponse { artifact })).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("Database error during artifact lookup: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn upload_artifact_content(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    body: Body,
) -> impl IntoResponse {
    let session = match state.artifact_repo.get_upload_session(id).await {
        Ok(Some(session)) => session,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("Database error during upload session lookup: {:?}", err);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let artifact = match state.artifact_repo.get_artifact(session.artifact_id).await {
        Ok(Some(artifact)) => artifact,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("Database error during artifact upload lookup: {:?}", err);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let bytes: Bytes = match to_bytes(body, artifact.size_bytes.max(0) as usize + 1).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!("Artifact upload body rejected: {}", err);
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    if bytes.len() as i64 != artifact.size_bytes {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let actual_digest = format!("sha256:{:x}", Sha256::digest(bytes.as_ref()));
    if !artifact_digest_matches(&artifact.digest, &actual_digest) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "artifact digest verification failed" })),
        )
            .into_response();
    }

    let key = format!("artifacts/{}", artifact.id);
    match storage::put_bytes(&state.artifact_store, &key, bytes.to_vec()).await {
        Ok(()) => match state.artifact_repo.complete_artifact_upload(id, &key).await {
            Ok(artifact) => (StatusCode::OK, Json(ArtifactResponse { artifact })).into_response(),
            Err(RepositoryError::Conflict(msg)) => {
                (StatusCode::CONFLICT, Json(json!({ "error": msg }))).into_response()
            }
            Err(RepositoryError::NotFound) => StatusCode::NOT_FOUND.into_response(),
            Err(err) => {
                tracing::error!("Failed to finalize artifact upload: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        Err(err) => {
            tracing::error!("Failed to upload artifact content: {}", err);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn artifact_digest_matches(expected: &str, actual: &str) -> bool {
    expected == actual || expected == actual.trim_start_matches("sha256:")
}

pub async fn heartbeat_executor(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    match state
        .executor_repo
        .record_heartbeat(payload.executor_id)
        .await
    {
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
        "Cancelled" => domain::TaskStatus::Cancelled,
        "TimedOut" => domain::TaskStatus::TimedOut,
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

pub async fn put_rate_limit(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PutRateLimitRequest>,
) -> impl IntoResponse {
    match state
        .rate_limit_repo
        .put_rate_limit(payload.tenant_id, &payload.key, payload.max_inflight)
        .await
    {
        Ok(_) => StatusCode::CREATED.into_response(),
        Err(RepositoryError::Conflict(msg)) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
        }
        Err(err) => {
            tracing::error!("Database error during rate-limit upsert: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn put_secret(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PutSecretRequest>,
) -> impl IntoResponse {
    match state
        .secret_repo
        .put_secret(payload.tenant_id, &payload.key, &payload.value)
        .await
    {
        Ok(_) => StatusCode::CREATED.into_response(),
        Err(err) => {
            tracing::error!("Database error during secret upsert: {:?}", err);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

enum BlobKind {
    Logs,
    Result,
}

async fn fetch_task_blob(state: Arc<AppState>, id: Uuid, kind: BlobKind) -> Response {
    let outputs = match state.task_repo.get_task_outputs(id).await {
        Ok(Some(outputs)) => outputs,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("Failed to load task outputs: {:?}", err);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let object_key = match kind {
        BlobKind::Logs => outputs.logs_ref.clone(),
        BlobKind::Result => outputs.result_ref,
    };

    if matches!(kind, BlobKind::Logs) && object_key.is_none() {
        match state.task_repo.list_task_log_chunks(id).await {
            Ok(chunks) if !chunks.is_empty() => {
                let mut response = Response::new(Body::from(chunks.join("")));
                *response.status_mut() = StatusCode::OK;
                response.headers_mut().insert(
                    CONTENT_TYPE,
                    HeaderValue::from_static("text/plain; charset=utf-8"),
                );
                return response;
            }
            Ok(_) => return StatusCode::NOT_FOUND.into_response(),
            Err(err) => {
                tracing::error!("Failed to load persisted log chunks: {:?}", err);
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    let object_key = match object_key {
        Some(key) => key,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let bytes = match state
        .output_store
        .get(&StorePath::from(object_key.as_str()))
        .await
    {
        Ok(result) => match result.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::error!("Failed reading task output body: {}", err);
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        },
        Err(object_store::Error::NotFound { .. }) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!("Failed fetching task output object: {}", err);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let content_type = match kind {
        BlobKind::Logs => HeaderValue::from_static("text/plain; charset=utf-8"),
        BlobKind::Result => HeaderValue::from_static("application/json"),
    };

    let mut response = Response::new(Body::from(bytes.to_vec()));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(CONTENT_TYPE, content_type);
    response
}

async fn load_task_payload(
    store: &Arc<dyn ObjectStore>,
    payload_ref: Option<&str>,
) -> Result<Option<Value>, String> {
    let Some(payload_ref) = payload_ref else {
        return Ok(None);
    };

    let result = store
        .get(&StorePath::from(payload_ref))
        .await
        .map_err(|err| err.to_string())?;
    let bytes = result.bytes().await.map_err(|err| err.to_string())?;
    let payload = serde_json::from_slice::<Value>(&bytes).map_err(|err| err.to_string())?;

    Ok(Some(payload))
}
