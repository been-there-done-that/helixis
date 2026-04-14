pub mod handlers;
pub mod live_logs;
pub mod storage;

use axum::{
    Router,
    routing::{get, post},
};
use handlers::{AppState, submit_task};
use std::sync::Arc;

pub fn app_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/v1/tasks", post(submit_task))
        .route("/v1/tasks/:id", get(handlers::get_task))
        .route("/v1/tasks/:id/logs", get(handlers::get_task_logs))
        .route("/v1/tasks/:id/logs/live", get(handlers::stream_task_logs))
        .route("/v1/tasks/:id/logs/append", post(handlers::append_task_log))
        .route("/v1/tasks/:id/result", get(handlers::get_task_result))
        .route("/v1/tasks/:id/cancel", post(handlers::cancel_task))
        .route("/v1/tasks/:id/status", post(handlers::update_task_status))
        .route(
            "/v1/artifact-uploads",
            post(handlers::create_artifact_upload),
        )
        .route("/v1/artifacts/:id", get(handlers::get_artifact))
        .route(
            "/v1/artifact-uploads/:id/content",
            post(handlers::upload_artifact_content),
        )
        .route("/v1/payload-uploads", post(handlers::create_payload_upload))
        .route(
            "/v1/payload-uploads/:id/content",
            post(handlers::upload_payload_content),
        )
        .route("/v1/secrets", post(handlers::put_secret))
        .route("/v1/rate-limits", post(handlers::put_rate_limit))
        .route("/v1/executors/register", post(handlers::register_executor))
        .route(
            "/v1/executors/heartbeat",
            post(handlers::heartbeat_executor),
        )
        .route("/v1/executors/poll", post(handlers::poll_tasks))
        .with_state(state)
}
