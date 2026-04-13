use application::ports::repositories::{RepositoryError, TaskRepository};
use async_trait::async_trait;
use domain::{Task, TaskLease, TaskStatus};
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresTaskRepository {
    pool: PgPool,
}

impl PostgresTaskRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TaskRepository for PostgresTaskRepository {
    async fn get_task(&self, id: Uuid) -> Result<Option<Task>, RepositoryError> {
        let row = sqlx::query!("SELECT * FROM tasks WHERE id = $1", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let status_str: String = row.status;
        let status = match status_str.as_str() {
            "Queued" => TaskStatus::Queued,
            "Scheduled" => TaskStatus::Scheduled,
            "Running" => TaskStatus::Running,
            "Succeeded" => TaskStatus::Succeeded,
            "Failed" => TaskStatus::Failed,
            "Cancelled" => TaskStatus::Cancelled,
            "TimedOut" => TaskStatus::TimedOut,
            "DeadLetter" => TaskStatus::DeadLetter,
            _ => TaskStatus::Failed,
        };

        Ok(Some(Task {
            id: row.id,
            tenant_id: row.tenant_id,
            artifact_id: row.artifact_id,
            runtime_pack_id: row.runtime_pack_id,
            status,
            priority: row.priority,
            rate_limit_key: row.rate_limit_key,
            timeout_seconds: row.timeout_seconds,
            max_attempts: row.max_attempts,
            current_attempt: row.current_attempt,
            idempotency_key: row.idempotency_key,
        }))
    }

    async fn insert_task(&self, task: &Task) -> Result<(), RepositoryError> {
        let status_str = match task.status {
            TaskStatus::Queued => "Queued",
            TaskStatus::Scheduled => "Scheduled",
            TaskStatus::Running => "Running",
            TaskStatus::Succeeded => "Succeeded",
            TaskStatus::Failed => "Failed",
            TaskStatus::Cancelled => "Cancelled",
            TaskStatus::TimedOut => "TimedOut",
            TaskStatus::DeadLetter => "DeadLetter",
        };

        let result = sqlx::query!(
            r#"
            INSERT INTO tasks (
                id,
                tenant_id,
                artifact_id,
                runtime_pack_id,
                status,
                priority,
                rate_limit_key,
                timeout_seconds,
                max_attempts,
                current_attempt,
                idempotency_key
            )
            SELECT
                $1,
                $2,
                $3,
                $4,
                $5,
                $6,
                $7,
                $8,
                $9,
                $10,
                $11
            WHERE EXISTS (
                SELECT 1
                FROM artifacts
                WHERE id = $3
                  AND tenant_id = $2
                  AND runtime_pack_id = $4
            )
            "#,
            task.id,
            task.tenant_id,
            task.artifact_id,
            task.runtime_pack_id,
            status_str,
            task.priority,
            task.rate_limit_key,
            task.timeout_seconds,
            task.max_attempts,
            task.current_attempt,
            task.idempotency_key
        )
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::Conflict(
                "artifact does not belong to tenant or runtime pack does not match".to_string(),
            ));
        }
        Ok(())
    }

    async fn poll_and_lease(
        &self,
        runtime_pack_id: &str,
        executor_id: Uuid,
        lease_duration_sec: i32,
    ) -> Result<Option<(Task, TaskLease)>, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        let row = sqlx::query!(
            r#"
            UPDATE tasks
            SET status = 'Scheduled', updated_at = NOW()
            WHERE id = (
                SELECT id FROM tasks 
                WHERE status = 'Queued' AND runtime_pack_id = $1 
                ORDER BY priority DESC, created_at ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            )
            RETURNING *
            "#,
            runtime_pack_id
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let task_id = row.id;
        let lease_id = Uuid::new_v4();

        // Auto-stub the executor session to prevent foreign-key failures in absence of proper heartbeat phase
        sqlx::query!(
            "INSERT INTO executors (id, session_id, capabilities_json) VALUES ($1, gen_random_uuid(), '[]'::jsonb) ON CONFLICT (id) DO NOTHING",
            executor_id
        ).execute(&mut *tx).await.map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        // Use a simple string building approach for the interval
        let query_str = format!(
            "INSERT INTO task_leases (id, task_id, executor_id, expires_at) VALUES ($1, $2, $3, NOW() + INTERVAL '{} seconds')",
            lease_duration_sec
        );
        sqlx::query(&query_str)
            .bind(lease_id)
            .bind(task_id)
            .bind(executor_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        let status = TaskStatus::Scheduled;

        let task = Task {
            id: row.id,
            tenant_id: row.tenant_id,
            artifact_id: row.artifact_id,
            runtime_pack_id: row.runtime_pack_id,
            status,
            priority: row.priority,
            rate_limit_key: row.rate_limit_key,
            timeout_seconds: row.timeout_seconds,
            max_attempts: row.max_attempts,
            current_attempt: row.current_attempt,
            idempotency_key: row.idempotency_key,
        };

        let lease = TaskLease {
            id: lease_id,
            task_id,
            executor_id,
        };

        Ok(Some((task, lease)))
    }

    async fn update_task_status(
        &self,
        id: Uuid,
        lease_id: Uuid,
        executor_id: Uuid,
        status: TaskStatus,
    ) -> Result<(), RepositoryError> {
        let status_str = match status {
            TaskStatus::Queued => "Queued",
            TaskStatus::Scheduled => "Scheduled",
            TaskStatus::Running => "Running",
            TaskStatus::Succeeded => "Succeeded",
            TaskStatus::Failed => "Failed",
            TaskStatus::Cancelled => "Cancelled",
            TaskStatus::TimedOut => "TimedOut",
            TaskStatus::DeadLetter => "DeadLetter",
        };

        let result = sqlx::query!(
            r#"
            UPDATE tasks
            SET status = $1, updated_at = NOW()
            WHERE id = $2
              AND EXISTS (
                  SELECT 1
                  FROM task_leases
                  WHERE id = $3
                    AND task_id = $2
                    AND executor_id = $4
                    AND expires_at > NOW()
              )
            "#,
            status_str,
            id,
            lease_id,
            executor_id
        )
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::Conflict(
                "no active lease matches this completion".to_string(),
            ));
        }

        Ok(())
    }
}
