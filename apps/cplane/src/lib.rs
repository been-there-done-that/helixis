pub mod handlers;

use axum::{routing::{get, post}, Router};
use handlers::{submit_task, AppState};
use std::sync::Arc;

pub fn app_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/tasks", post(submit_task))
        .route("/v1/tasks/:id", get(handlers::get_task))
        .route("/v1/tasks/:id/status", post(handlers::update_task_status))
        .route("/v1/executors/poll", post(handlers::poll_tasks))
        .with_state(state)
}
