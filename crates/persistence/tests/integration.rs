use application::ports::repositories::{ExecutorRepository, TaskRepository};
use domain::{Artifact, Executor, RuntimePack, Task, TaskStatus};
use persistence::{
    db,
    repositories::{executor_repo::PostgresExecutorRepository, task_repo::PostgresTaskRepository},
};
use sqlx::PgPool;
use std::env;
use uuid::Uuid;

async fn setup_db() -> PgPool {
    // Try dotenv, ignore if not found
    let _ = dotenvy::dotenv();
    let db_url = env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/helixis".into());
    db::create_pool(&db_url).await.expect("Failed to connect to pool")
}

async fn insert_prereqs(pool: &PgPool) -> (Uuid, Uuid, String) {
    let tenant_id = Uuid::new_v4();
    let artifact_id = Uuid::new_v4();
    let runtime_pack_id = "python-3.11-v1".to_string();

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

    sqlx::query!(
        "INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes) VALUES ($1, $2, 'sha256:dummy', $3, 'main.py', 100)",
        artifact_id, tenant_id, runtime_pack_id
    )
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
        timeout_seconds: 300,
        max_attempts: 3,
        current_attempt: 0,
        idempotency_key: None,
    };

    task_repo.insert_task(&task).await.unwrap();

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
