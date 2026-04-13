pub mod handlers;

use axum::{routing::{get, post}, Router};
use handlers::{get_task, poll_tasks, submit_task, AppState};
use std::sync::Arc;

pub fn app_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/tasks", post(submit_task))
        .route("/v1/tasks/:id", get(get_task))
        .route("/v1/executors/poll", post(poll_tasks))
        .with_state(state)
}
