use application::ports::repositories::{
    ArtifactRepository, ExecutorRepository, RateLimitRepository, SecretRepository, TaskRepository,
};
use domain::{ArtifactStatus, ArtifactUploadStatus, Executor, Task, TaskStatus};
use persistence::{
    db,
    repositories::{
        artifact::PostgresArtifactRepository, executor::PostgresExecutorRepository,
        rate_limit::PostgresRateLimitRepository, secret::PostgresSecretRepository,
        task::PostgresTaskRepository,
    },
};
use sqlx::PgPool;
use std::env;
use uuid::Uuid;

async fn setup_db() -> PgPool {
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

async fn insert_prereqs(pool: &PgPool) -> (Uuid, Uuid, String) {
    let tenant_id = Uuid::new_v4();
    let artifact_id = Uuid::new_v4();
    let runtime_pack_id = format!("python-3.11-v1-{}", Uuid::new_v4()); // Ensure uniqueness per test if run concurrently

    sqlx::query!(
        "INSERT INTO tenants (id, name) VALUES ($1, 'Test Tenant')",
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

    sqlx::query(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes, status, object_key) VALUES ($1, $2, $3, $4, 'main.py', 100, 'Ready', $5)",
    )
    .bind(artifact_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4().to_string())
    .bind(&runtime_pack_id)
    .bind(format!("artifacts/{artifact_id}"))
    .execute(pool)
    .await
    .unwrap();

    (tenant_id, artifact_id, runtime_pack_id)
}

#[tokio::test]
async fn test_insert_and_poll() {
    let pool = setup_db().await;
    let task_repo = PostgresTaskRepository::new(pool.clone());
    let executor_repo = PostgresExecutorRepository::new(pool.clone());

    let (tenant_id, artifact_id, runtime_pack_id) = insert_prereqs(&pool).await;

    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack_id.clone()],
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    let task_id = Uuid::new_v4();
    let task = Task {
        id: task_id,
        tenant_id,
        artifact_id,
        runtime_pack_id: runtime_pack_id.clone(),
        status: TaskStatus::Queued,
        priority: 10,
        rate_limit_key: None,
        payload_ref: None,
        timeout_seconds: 300,
        max_attempts: 3,
        current_attempt: 0,
        idempotency_key: None,
        payload_size_bytes: None,
    };

    task_repo.insert_task(&task).await.unwrap();

    // Verify task fetch
    let fetched = task_repo.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(fetched.id, task_id);

    // Poll the task
    let (polled_task, lease) = task_repo
        .poll_and_lease(&runtime_pack_id, executor.id, 60)
        .await
        .unwrap()
        .expect("Should poll a task");

    assert_eq!(polled_task.id, task_id);
    assert_eq!(polled_task.status, TaskStatus::Scheduled);
    assert_eq!(lease.task_id, task_id);
    assert_eq!(lease.executor_id, executor.id);

    // Poll again, should be empty because it is SKIP LOCKED/Scheduled
    let empty_poll = task_repo
        .poll_and_lease(&runtime_pack_id, executor.id, 60)
        .await
        .unwrap();
    assert!(empty_poll.is_none());
}

#[tokio::test]
async fn test_executor_heartbeat() {
    let pool = setup_db().await;
    let repo = PostgresExecutorRepository::new(pool.clone());

    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec!["python-3.11".to_string()],
    };

    repo.upsert_executor(&executor)
        .await
        .expect("Failed to upsert executor");
    repo.record_heartbeat(executor.id)
        .await
        .expect("Failed to record heartbeat");
}

#[tokio::test]
async fn test_get_not_found() {
    let pool = setup_db().await;
    let repo = PostgresTaskRepository::new(pool.clone());

    let res = repo.get_task(Uuid::new_v4()).await.unwrap();
    assert!(res.is_none());
}

#[tokio::test]
async fn test_requeue_expired_lease() {
    let pool = setup_db().await;
    let task_repo = PostgresTaskRepository::new(pool.clone());
    let executor_repo = PostgresExecutorRepository::new(pool.clone());

    let (tenant_id, artifact_id, runtime_pack_id) = insert_prereqs(&pool).await;

    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack_id.clone()],
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    let task = Task {
        id: Uuid::new_v4(),
        tenant_id,
        artifact_id,
        runtime_pack_id: runtime_pack_id.clone(),
        status: TaskStatus::Queued,
        priority: 1,
        rate_limit_key: None,
        payload_ref: None,
        timeout_seconds: 60,
        max_attempts: 3,
        current_attempt: 0,
        idempotency_key: None,
        payload_size_bytes: None,
    };

    task_repo.insert_task(&task).await.unwrap();

    let (_polled_task, lease) = task_repo
        .poll_and_lease(&runtime_pack_id, executor.id, 1)
        .await
        .unwrap()
        .unwrap();

    task_repo
        .update_task_status(
            task.id,
            lease.id,
            executor.id,
            TaskStatus::Running,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    sqlx::query!(
        "UPDATE task_leases SET expires_at = NOW() - INTERVAL '1 second' WHERE id = $1",
        lease.id
    )
    .execute(&pool)
    .await
    .unwrap();

    let requeued = task_repo.requeue_expired_leases().await.unwrap();
    assert!(requeued >= 1);

    let fetched = task_repo.get_task(task.id).await.unwrap().unwrap();
    assert_eq!(fetched.status, TaskStatus::Queued);
}

#[tokio::test]
async fn test_stale_executor_cannot_poll() {
    let pool = setup_db().await;
    let task_repo = PostgresTaskRepository::new(pool.clone());
    let executor_repo = PostgresExecutorRepository::new(pool.clone());

    let (tenant_id, artifact_id, runtime_pack_id) = insert_prereqs(&pool).await;

    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack_id.clone()],
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    sqlx::query!(
        "UPDATE executors SET last_heartbeat_at = NOW() - INTERVAL '5 minutes' WHERE id = $1",
        executor.id
    )
    .execute(&pool)
    .await
    .unwrap();

    let task = Task {
        id: Uuid::new_v4(),
        tenant_id,
        artifact_id,
        runtime_pack_id,
        status: TaskStatus::Queued,
        priority: 1,
        rate_limit_key: None,
        payload_ref: None,
        timeout_seconds: 60,
        max_attempts: 3,
        current_attempt: 0,
        idempotency_key: None,
        payload_size_bytes: None,
    };

    task_repo.insert_task(&task).await.unwrap();

    let err = task_repo
        .poll_and_lease(&task.runtime_pack_id, executor.id, 60)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        application::ports::repositories::RepositoryError::Conflict(_)
    ));
}

#[tokio::test]
async fn test_rate_limit_prevents_second_lease() {
    let pool = setup_db().await;
    let task_repo = PostgresTaskRepository::new(pool.clone());
    let executor_repo = PostgresExecutorRepository::new(pool.clone());
    let rate_limit_repo = PostgresRateLimitRepository::new(pool.clone());

    let (tenant_id, artifact_id, runtime_pack_id) = insert_prereqs(&pool).await;

    rate_limit_repo
        .put_rate_limit(tenant_id, "customer-123", 1)
        .await
        .unwrap();

    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack_id.clone()],
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    for _ in 0..2 {
        let task = Task {
            id: Uuid::new_v4(),
            tenant_id,
            artifact_id,
            runtime_pack_id: runtime_pack_id.clone(),
            status: TaskStatus::Queued,
            priority: 1,
            rate_limit_key: Some("customer-123".to_string()),
            payload_ref: None,
            timeout_seconds: 60,
            max_attempts: 3,
            current_attempt: 0,
            idempotency_key: None,
            payload_size_bytes: None,
        };

        task_repo.insert_task(&task).await.unwrap();
    }

    let first = task_repo
        .poll_and_lease(&runtime_pack_id, executor.id, 60)
        .await
        .unwrap();
    assert!(first.is_some());

    let second = task_repo
        .poll_and_lease(&runtime_pack_id, executor.id, 60)
        .await
        .unwrap();
    assert!(second.is_none());
}

#[tokio::test]
async fn test_secret_round_trip() {
    let pool = setup_db().await;
    let (tenant_id, _, _) = insert_prereqs(&pool).await;
    let secret_repo = PostgresSecretRepository::new(pool.clone(), "test-secret-key");

    secret_repo
        .put_secret(tenant_id, "API_TOKEN", "top-secret")
        .await
        .unwrap();

    let secrets = secret_repo.get_tenant_secrets(tenant_id).await.unwrap();
    assert_eq!(secrets.get("API_TOKEN"), Some(&"top-secret".to_string()));
}

#[tokio::test]
async fn test_artifact_round_trip() {
    let pool = setup_db().await;
    let (tenant_id, _, runtime_pack_id) = insert_prereqs(&pool).await;
    let artifact_repo = PostgresArtifactRepository::new(pool.clone());

    let artifact = domain::Artifact {
        id: Uuid::new_v4(),
        tenant_id,
        digest: format!("sha256:{}", Uuid::new_v4()),
        runtime_pack_id,
        entrypoint: "main.py".to_string(),
        size_bytes: 42,
        status: ArtifactStatus::PendingUpload,
        object_key: None,
    };

    let session = artifact_repo
        .create_artifact_upload(&artifact)
        .await
        .unwrap();
    assert_eq!(session.status, ArtifactUploadStatus::Pending);

    let fetched = artifact_repo
        .get_artifact(artifact.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.status, ArtifactStatus::PendingUpload);
    assert_eq!(fetched.digest, artifact.digest);

    let completed = artifact_repo
        .complete_artifact_upload(session.id, "artifacts/test")
        .await
        .unwrap();
    assert_eq!(completed.status, ArtifactStatus::Ready);
    assert_eq!(completed.object_key.as_deref(), Some("artifacts/test"));
}

#[tokio::test]
async fn test_get_task_outputs() {
    let pool = setup_db().await;
    let task_repo = PostgresTaskRepository::new(pool.clone());
    let executor_repo = PostgresExecutorRepository::new(pool.clone());

    let (tenant_id, artifact_id, runtime_pack_id) = insert_prereqs(&pool).await;
    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack_id.clone()],
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    let task = Task {
        id: Uuid::new_v4(),
        tenant_id,
        artifact_id,
        runtime_pack_id: runtime_pack_id.clone(),
        status: TaskStatus::Queued,
        priority: 1,
        rate_limit_key: None,
        payload_ref: None,
        timeout_seconds: 60,
        max_attempts: 1,
        current_attempt: 0,
        idempotency_key: None,
        payload_size_bytes: None,
    };
    task_repo.insert_task(&task).await.unwrap();

    let (_polled, lease) = task_repo
        .poll_and_lease(&runtime_pack_id, executor.id, 60)
        .await
        .unwrap()
        .unwrap();

    task_repo
        .update_task_status(
            task.id,
            lease.id,
            executor.id,
            TaskStatus::Running,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    task_repo
        .update_task_status(
            task.id,
            lease.id,
            executor.id,
            TaskStatus::TimedOut,
            Some("tasks/test/logs.txt".to_string()),
            Some("tasks/test/result.json".to_string()),
            Some("timed out".to_string()),
        )
        .await
        .unwrap();

    let outputs = task_repo.get_task_outputs(task.id).await.unwrap().unwrap();
    assert_eq!(outputs.status, TaskStatus::TimedOut);
    assert_eq!(outputs.logs_ref.as_deref(), Some("tasks/test/logs.txt"));
    assert_eq!(
        outputs.result_ref.as_deref(),
        Some("tasks/test/result.json")
    );
}

#[tokio::test]
async fn test_append_and_list_task_log_chunks() {
    let pool = setup_db().await;
    let task_repo = PostgresTaskRepository::new(pool.clone());
    let executor_repo = PostgresExecutorRepository::new(pool.clone());

    let (tenant_id, artifact_id, runtime_pack_id) = insert_prereqs(&pool).await;
    let executor = Executor {
        id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        capabilities: vec![runtime_pack_id.clone()],
    };
    executor_repo.upsert_executor(&executor).await.unwrap();

    let task = Task {
        id: Uuid::new_v4(),
        tenant_id,
        artifact_id,
        runtime_pack_id: runtime_pack_id.clone(),
        status: TaskStatus::Queued,
        priority: 1,
        rate_limit_key: None,
        payload_ref: None,
        timeout_seconds: 60,
        max_attempts: 1,
        current_attempt: 0,
        idempotency_key: None,
        payload_size_bytes: None,
    };
    task_repo.insert_task(&task).await.unwrap();

    let (_polled, lease) = task_repo
        .poll_and_lease(&runtime_pack_id, executor.id, 60)
        .await
        .unwrap()
        .unwrap();

    task_repo
        .append_task_log_chunk(task.id, lease.id, executor.id, "stdout", "hello")
        .await
        .unwrap();
    task_repo
        .append_task_log_chunk(task.id, lease.id, executor.id, "stderr", "world")
        .await
        .unwrap();

    let chunks = task_repo.list_task_log_chunks(task.id).await.unwrap();
    assert_eq!(chunks, vec!["[stdout] hello", "[stderr] world"]);
}
