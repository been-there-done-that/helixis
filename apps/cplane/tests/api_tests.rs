use application::ports::repositories::ExecutorRepository;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cplane::{app_router, handlers::AppState, live_logs::LiveLogHub};
use domain::{Executor, TaskStatus};
use http_body_util::BodyExt;
use object_store::ObjectStore;
use object_store::memory::InMemory;
use persistence::{
    db,
    repositories::{
        artifact::PostgresArtifactRepository, executor::PostgresExecutorRepository,
        rate_limit::PostgresRateLimitRepository, secret::PostgresSecretRepository,
        task::PostgresTaskRepository,
    },
};
use protocol::api::{
    ArtifactRegisterRequest, ArtifactResponse, HeartbeatRequest, LiveLogChunkRequest, PollRequest,
    PollResponse, PutRateLimitRequest, PutSecretRequest, RegisterExecutorRequest, TaskResponse,
    TaskStatusUpdateRequest, TaskSubmitRequest,
};
use std::env;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    let _ = dotenvy::dotenv();
    let db_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/helixis".into());
    let pool = db::create_pool(&db_url)
        .await
        .expect("Failed to connect to pool");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
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

fn build_state(pool: &sqlx::PgPool) -> Arc<AppState> {
    Arc::new(AppState {
        task_repo: Arc::new(PostgresTaskRepository::new(pool.clone())),
        artifact_repo: Arc::new(PostgresArtifactRepository::new(pool.clone())),
        executor_repo: Arc::new(PostgresExecutorRepository::new(pool.clone())),
        rate_limit_repo: Arc::new(PostgresRateLimitRepository::new(pool.clone())),
        secret_repo: Arc::new(PostgresSecretRepository::new(
            pool.clone(),
            "test-secret-key",
        )),
        output_store: Arc::new(InMemory::new()),
        artifact_store: Arc::new(InMemory::new()),
        log_hub: Arc::new(LiveLogHub::new()),
    })
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

    let state = build_state(&pool);
    let executor_repo = Arc::new(PostgresExecutorRepository::new(pool.clone()));
    let app = app_router(state);

    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack.clone()],
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    // 1. Submit POST
    let submit_req = TaskSubmitRequest {
        tenant_id,
        artifact_id,
        runtime_pack_id: runtime_pack.clone(),
        payload: None,
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
    assert!(body_json.environment.is_empty());
    assert!(body_json.payload.is_none());
    let polled_task = body_json.task.unwrap();
    assert_eq!(polled_task.id, task_id);
    assert_eq!(polled_task.status, TaskStatus::Scheduled);

    assert!(body_json.lease.is_some());
    let lease = body_json.lease.unwrap();
    assert_eq!(lease.executor_id, executor.id);

    let running_req = TaskStatusUpdateRequest {
        status: "Running".to_string(),
        lease_id: lease.id,
        executor_id: executor.id,
        logs_ref: None,
        result_ref: None,
        last_error_message: None,
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/tasks/{task_id}/status"))
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&running_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let status_req = TaskStatusUpdateRequest {
        status: "Succeeded".to_string(),
        lease_id: lease.id,
        executor_id: executor.id,
        logs_ref: None,
        result_ref: None,
        last_error_message: None,
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/tasks/{task_id}/status"))
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&status_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_reject_mismatched_artifact_runtime() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;
    let other_runtime_pack = format!("node-20-v1-api-{}", Uuid::new_v4());

    sqlx::query!(
        "INSERT INTO runtime_packs (id, language, language_version, sandbox_kind) VALUES ($1, 'node', '20', 'proc')",
        other_runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let artifact_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, $3, $4, 'main.py', 100)",
        artifact_id, tenant_id, Uuid::new_v4().to_string(), runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let app = app_router(build_state(&pool));

    let submit_req = TaskSubmitRequest {
        tenant_id,
        artifact_id,
        runtime_pack_id: other_runtime_pack,
        payload: None,
        priority: Some(5),
        rate_limit_key: None,
        timeout_seconds: Some(100),
        max_attempts: Some(2),
        idempotency_key: None,
    };

    let response = app
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

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_register_and_heartbeat_endpoints() {
    let pool = setup_db().await;
    let app = app_router(build_state(&pool));

    let executor_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let register_req = RegisterExecutorRequest {
        executor_id,
        session_id,
        capabilities: vec!["python-3.11-v1".to_string()],
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/executors/register")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&register_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let heartbeat_req = HeartbeatRequest { executor_id };
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/executors/heartbeat")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&heartbeat_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_cancel_endpoint_marks_task_cancelled() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;

    let artifact_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, $3, $4, 'main.py', 100)",
        artifact_id, tenant_id, Uuid::new_v4().to_string(), runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let app = app_router(build_state(&pool));

    let submit_req = TaskSubmitRequest {
        tenant_id,
        artifact_id,
        runtime_pack_id: runtime_pack,
        payload: None,
        priority: Some(1),
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

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_json: TaskResponse = serde_json::from_slice(&body_bytes).unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/tasks/{}/cancel", body_json.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/tasks/{}", body_json.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let task_json: TaskResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(task_json.status, TaskStatus::Cancelled);
}

#[tokio::test]
async fn test_poll_includes_tenant_secrets() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;
    let artifact_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, $3, $4, 'main.py', 100)",
        artifact_id, tenant_id, Uuid::new_v4().to_string(), runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let app = app_router(build_state(&pool));
    let executor_id = Uuid::new_v4();

    let register_req = RegisterExecutorRequest {
        executor_id,
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack.clone()],
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/executors/register")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&register_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let secret_req = PutSecretRequest {
        tenant_id,
        key: "API_TOKEN".to_string(),
        value: "top-secret".to_string(),
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/secrets")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&secret_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let submit_req = TaskSubmitRequest {
        tenant_id,
        artifact_id,
        runtime_pack_id: runtime_pack.clone(),
        payload: None,
        priority: Some(1),
        rate_limit_key: None,
        timeout_seconds: Some(30),
        max_attempts: Some(1),
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

    let poll_req = PollRequest {
        runtime_pack_id: runtime_pack,
        executor_id,
        lease_duration_sec: 60,
    };

    let response = app
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
    assert_eq!(
        body_json.environment.get("API_TOKEN"),
        Some(&"top-secret".to_string())
    );
}

#[tokio::test]
async fn test_rate_limit_blocks_second_inflight_task() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;
    let artifact_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, $3, $4, 'main.py', 100)",
        artifact_id, tenant_id, Uuid::new_v4().to_string(), runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let app = app_router(build_state(&pool));
    let executor_id = Uuid::new_v4();

    let register_req = RegisterExecutorRequest {
        executor_id,
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack.clone()],
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/executors/register")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&register_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let limit_req = PutRateLimitRequest {
        tenant_id,
        key: "customer-123".to_string(),
        max_inflight: 1,
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/rate-limits")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&limit_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    for _ in 0..2 {
        let submit_req = TaskSubmitRequest {
            tenant_id,
            artifact_id,
            runtime_pack_id: runtime_pack.clone(),
            payload: None,
            priority: Some(1),
            rate_limit_key: Some("customer-123".to_string()),
            timeout_seconds: Some(30),
            max_attempts: Some(1),
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
    }

    let poll_req = PollRequest {
        runtime_pack_id: runtime_pack.clone(),
        executor_id,
        lease_duration_sec: 60,
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
    let first_poll: PollResponse = serde_json::from_slice(&body).unwrap();
    assert!(first_poll.task.is_some());

    let response = app
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
    let second_poll: PollResponse = serde_json::from_slice(&body).unwrap();
    assert!(second_poll.task.is_none());
}

#[tokio::test]
async fn test_register_and_get_artifact() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;
    let app = app_router(build_state(&pool));

    let request = ArtifactRegisterRequest {
        tenant_id,
        digest: format!("sha256:{}", Uuid::new_v4()),
        runtime_pack_id: runtime_pack,
        entrypoint: "main.py".to_string(),
        size_bytes: 512,
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/artifacts")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let artifact_response: ArtifactResponse = serde_json::from_slice(&body).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/artifacts/{}", artifact_response.artifact.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_upload_artifact_content_stores_blob() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;
    let artifact_store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let state = Arc::new(AppState {
        task_repo: Arc::new(PostgresTaskRepository::new(pool.clone())),
        artifact_repo: Arc::new(PostgresArtifactRepository::new(pool.clone())),
        executor_repo: Arc::new(PostgresExecutorRepository::new(pool.clone())),
        rate_limit_repo: Arc::new(PostgresRateLimitRepository::new(pool.clone())),
        secret_repo: Arc::new(PostgresSecretRepository::new(
            pool.clone(),
            "test-secret-key",
        )),
        output_store: Arc::new(InMemory::new()),
        artifact_store: Arc::clone(&artifact_store),
        log_hub: Arc::new(LiveLogHub::new()),
    });
    let app = app_router(state);

    let register = ArtifactRegisterRequest {
        tenant_id,
        digest: format!("sha256:{}", Uuid::new_v4()),
        runtime_pack_id: runtime_pack,
        entrypoint: "main.py".to_string(),
        size_bytes: 4,
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/artifacts")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&register).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let artifact_response: ArtifactResponse = serde_json::from_slice(&body).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/artifacts/{}/content",
                    artifact_response.artifact.id
                ))
                .body(Body::from("test"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let bytes = artifact_store
        .get(&object_store::path::Path::from(
            artifact_response.artifact.id.to_string(),
        ))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"test");
}

#[tokio::test]
async fn test_task_output_retrieval_endpoints() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;
    let output_store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let state = Arc::new(AppState {
        task_repo: Arc::new(PostgresTaskRepository::new(pool.clone())),
        artifact_repo: Arc::new(PostgresArtifactRepository::new(pool.clone())),
        executor_repo: Arc::new(PostgresExecutorRepository::new(pool.clone())),
        rate_limit_repo: Arc::new(PostgresRateLimitRepository::new(pool.clone())),
        secret_repo: Arc::new(PostgresSecretRepository::new(
            pool.clone(),
            "test-secret-key",
        )),
        output_store: output_store.clone(),
        artifact_store: Arc::new(InMemory::new()),
        log_hub: Arc::new(LiveLogHub::new()),
    });
    let app = app_router(state);

    let artifact_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, $3, $4, 'main.py', 100)",
        artifact_id, tenant_id, Uuid::new_v4().to_string(), runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let executor_repo = Arc::new(PostgresExecutorRepository::new(pool.clone()));
    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack.clone()],
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    let task_id = Uuid::new_v4();
    sqlx::query!(
        r#"
        INSERT INTO tasks (
            id, tenant_id, artifact_id, runtime_pack_id, status, timeout_seconds, max_attempts, current_attempt, result_ref
        ) VALUES ($1, $2, $3, $4, 'Succeeded', 30, 1, 1, $5)
        "#,
        task_id,
        tenant_id,
        artifact_id,
        runtime_pack,
        "tasks/test/result.json"
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query!(
        r#"
        INSERT INTO task_attempts (id, task_id, executor_id, lease_id, exit_reason, logs_ref)
        VALUES ($1, $2, $3, $4, 'Succeeded', $5)
        "#,
        Uuid::new_v4(),
        task_id,
        executor.id,
        Uuid::new_v4(),
        "tasks/test/logs.txt"
    )
    .execute(&pool)
    .await
    .unwrap();

    cplane::storage::put_bytes(&output_store, "tasks/test/logs.txt", b"hello logs".to_vec())
        .await
        .unwrap();
    cplane::storage::put_bytes(
        &output_store,
        "tasks/test/result.json",
        br#"{"ok":true}"#.to_vec(),
    )
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/tasks/{task_id}/logs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"hello logs");

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/tasks/{task_id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], br#"{"ok":true}"#);
}

#[tokio::test]
async fn test_poll_includes_offloaded_payload() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;
    let artifact_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, $3, $4, 'main.py', 100)",
        artifact_id, tenant_id, Uuid::new_v4().to_string(), runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let app = app_router(build_state(&pool));
    let executor_id = Uuid::new_v4();
    let register_req = RegisterExecutorRequest {
        executor_id,
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack.clone()],
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/executors/register")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&register_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let submit_req = TaskSubmitRequest {
        tenant_id,
        artifact_id,
        runtime_pack_id: runtime_pack.clone(),
        payload: Some(serde_json::json!({"message":"hello","n":1})),
        priority: Some(1),
        rate_limit_key: None,
        timeout_seconds: Some(30),
        max_attempts: Some(1),
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

    let poll_req = PollRequest {
        runtime_pack_id: runtime_pack,
        executor_id,
        lease_duration_sec: 60,
    };

    let response = app
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
    let poll: PollResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        poll.payload,
        Some(serde_json::json!({"message":"hello","n":1}))
    );
}

#[tokio::test]
async fn test_append_live_log_publishes_to_hub() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;
    let artifact_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, $3, $4, 'main.py', 100)",
        artifact_id, tenant_id, Uuid::new_v4().to_string(), runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let state = build_state(&pool);
    let mut receiver = state.log_hub.subscribe(task_id).await;
    let app = app_router(Arc::clone(&state));
    let executor_repo = PostgresExecutorRepository::new(pool.clone());
    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec!["python-3.11".to_string()],
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    sqlx::query!(
        r#"
        INSERT INTO tasks (
            id, tenant_id, artifact_id, runtime_pack_id, status, timeout_seconds, max_attempts, current_attempt
        ) VALUES ($1, $2, $3, $4, 'Running', 30, 1, 1)
        "#,
        task_id,
        tenant_id,
        artifact_id,
        runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let lease_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO task_leases (id, task_id, executor_id, expires_at) VALUES ($1, $2, $3, NOW() + INTERVAL '1 minute')",
        lease_id,
        task_id,
        executor.id
    )
    .execute(&pool)
    .await
    .unwrap();

    let request = LiveLogChunkRequest {
        lease_id,
        executor_id: executor.id,
        stream: "stdout".to_string(),
        chunk: "stream line".to_string(),
    };

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/tasks/{}/logs/append", task_id))
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let received = receiver.recv().await.unwrap();
    assert_eq!(received, "[stdout] stream line");
}

#[tokio::test]
async fn test_get_logs_replays_persisted_chunks() {
    let pool = setup_db().await;
    let (tenant_id, runtime_pack) = insert_prereqs(&pool).await;
    let artifact_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, $3, $4, 'main.py', 100)",
        artifact_id, tenant_id, Uuid::new_v4().to_string(), runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let state = build_state(&pool);
    let app = app_router(Arc::clone(&state));

    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack.clone()],
    };
    let executor_repo = PostgresExecutorRepository::new(pool.clone());
    executor_repo.upsert_executor(&executor).await.unwrap();

    let task_id = Uuid::new_v4();
    sqlx::query!(
        r#"
        INSERT INTO tasks (
            id, tenant_id, artifact_id, runtime_pack_id, status, timeout_seconds, max_attempts, current_attempt
        ) VALUES ($1, $2, $3, $4, 'Running', 30, 1, 1)
        "#,
        task_id,
        tenant_id,
        artifact_id,
        runtime_pack
    )
    .execute(&pool)
    .await
    .unwrap();

    let lease_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO task_leases (id, task_id, executor_id, expires_at) VALUES ($1, $2, $3, NOW() + INTERVAL '1 minute')",
        lease_id,
        task_id,
        executor.id
    )
    .execute(&pool)
    .await
    .unwrap();

    let request = LiveLogChunkRequest {
        lease_id,
        executor_id: executor.id,
        stream: "stdout".to_string(),
        chunk: "replay me".to_string(),
    };

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/tasks/{task_id}/logs/append"))
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/tasks/{task_id}/logs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"[stdout] replay me");
}
