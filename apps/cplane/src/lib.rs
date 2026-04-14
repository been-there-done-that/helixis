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
        .route("/v1/artifacts", post(handlers::register_artifact))
        .route("/v1/artifacts/:id", get(handlers::get_artifact))
        .route(
            "/v1/artifacts/:id/content",
            post(handlers::upload_artifact_content),
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
