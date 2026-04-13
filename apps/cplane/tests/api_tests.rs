use application::ports::repositories::ExecutorRepository;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cplane::{app_router, handlers::AppState};
use domain::{Executor, TaskStatus};
use http_body_util::BodyExt;
use persistence::{
    db,
    repositories::{executor_repo::PostgresExecutorRepository, task_repo::PostgresTaskRepository},
};
use protocol::api::{PollRequest, PollResponse, TaskResponse, TaskSubmitRequest};
use std::env;
use std::sync::Arc;
use tower::{Service, ServiceExt};
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    let _ = dotenvy::dotenv();
    let db_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/helixis".into());
    db::create_pool(&db_url)
        .await
        .expect("Failed to connect to pool")
}

async fn insert_prereqs(pool: &sqlx::PgPool) -> (Uuid, String) {
    let tenant_id = Uuid::new_v4();
    let runtime_pack_id = format!("python-3.11-v1-api-{}", Uuid::new_v4());

    sqlx::query!(
        "INSERT INTO tenants (id, name) VALUES ($1, 'API Test Tenant')",
        tenant_id
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query!(
        "INSERT INTO runtime_packs (id, language, language_version, sandbox_kind) VALUES ($1, 'python', '3.11', 'proc')",
        runtime_pack_id
    )
    .execute(pool)
    .await
    .unwrap();

    (tenant_id, runtime_pack_id)
}

#[tokio::test]
async fn test_full_api_flow() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;

    // We need an artifact
    let artifact_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, $3, $4, 'main.py', 100)",
        artifact_id, tenant_id, Uuid::new_v4().to_string(), runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let task_repo = Arc::new(PostgresTaskRepository::new(pool.clone()));
    let executor_repo = PostgresExecutorRepository::new(pool.clone());
    let state = Arc::new(AppState { task_repo });
    let mut app = app_router(state);

    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    // 1. Submit POST
    let submit_req = TaskSubmitRequest {
        tenant_id,
        artifact_id,
        runtime_pack_id: runtime_pack.clone(),
        priority: Some(5),
        rate_limit_key: None,
        timeout_seconds: Some(100),
        max_attempts: Some(2),
        idempotency_key: None,
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tasks")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&submit_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_json: TaskResponse = serde_json::from_slice(&body_bytes).unwrap();
    let task_id = body_json.id;

    assert_eq!(body_json.status, TaskStatus::Queued);

    // 2. Poll POST
    let poll_req = PollRequest {
        runtime_pack_id: runtime_pack.clone(),
        executor_id: executor.id,
        lease_duration_sec: 120,
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/executors/poll")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&poll_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body_json: PollResponse = serde_json::from_slice(&body).unwrap();

    assert!(body_json.task.is_some());
    let polled_task = body_json.task.unwrap();
    assert_eq!(polled_task.id, task_id);
    assert_eq!(polled_task.status, TaskStatus::Scheduled);

    assert!(body_json.lease.is_some());
    let lease = body_json.lease.unwrap();
    assert_eq!(lease.executor_id, executor.id);
}
