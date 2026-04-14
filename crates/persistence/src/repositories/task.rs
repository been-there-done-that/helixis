use application::ports::repositories::{RepositoryError, TaskRepository};
use async_trait::async_trait;
use domain::{Task, TaskLease, TaskOutputs, TaskStatus};
use sqlx::{PgPool, Row};
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
            payload_ref: row.payload_ref,
            timeout_seconds: row.timeout_seconds,
            max_attempts: row.max_attempts,
            current_attempt: row.current_attempt,
            idempotency_key: row.idempotency_key,
            payload_size_bytes: row.payload_size_bytes,
        }))
    }

    async fn get_task_outputs(&self, id: Uuid) -> Result<Option<TaskOutputs>, RepositoryError> {
        let row = sqlx::query!(
            r#"
            SELECT
                t.id,
                t.status,
                t.result_ref,
                (
                    SELECT ta.logs_ref
                    FROM task_attempts ta
                    WHERE ta.task_id = t.id
                    ORDER BY ta.started_at DESC
                    LIMIT 1
                ) AS logs_ref
            FROM tasks t
            WHERE t.id = $1
            "#,
            id
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        let row = match row {
            Some(row) => row,
            None => return Ok(None),
        };

        let status = match row.status.as_str() {
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

        Ok(Some(TaskOutputs {
            task_id: row.id,
            status,
            logs_ref: row.logs_ref,
            result_ref: row.result_ref,
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
                payload_ref,
                timeout_seconds,
                max_attempts,
                current_attempt,
                idempotency_key,
                payload_size_bytes
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
                $11,
                $12,
                $13
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
            task.payload_ref,
            task.timeout_seconds,
            task.max_attempts,
            task.current_attempt,
            task.idempotency_key,
            task.payload_size_bytes
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

        let executor_exists = sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM executors WHERE id = $1)",
            executor_id
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?
        .unwrap_or(false);

        if !executor_exists {
            return Err(RepositoryError::Conflict(
                "executor must register before polling".to_string(),
            ));
        }

        let executor_fresh = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM executors
                WHERE id = $1
                  AND last_heartbeat_at >= NOW() - INTERVAL '60 seconds'
            )
            "#,
            executor_id
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?
        .unwrap_or(false);

        if !executor_fresh {
            return Err(RepositoryError::Conflict(
                "executor heartbeat is stale".to_string(),
            ));
        }

        let row = sqlx::query(
            r#"
            UPDATE tasks
            SET status = 'Scheduled', scheduled_at = NOW(), updated_at = NOW()
            WHERE id = (
                SELECT candidate.id
                FROM tasks candidate
                WHERE candidate.status = 'Queued'
                  AND candidate.runtime_pack_id = $1
                  AND (
                    candidate.rate_limit_key IS NULL
                    OR NOT EXISTS (
                        SELECT 1
                        FROM rate_limits rl
                        WHERE rl.tenant_id = candidate.tenant_id
                          AND rl.rate_limit_key = candidate.rate_limit_key
                          AND (
                            SELECT COUNT(*)
                            FROM tasks active
                            WHERE active.tenant_id = candidate.tenant_id
                              AND active.rate_limit_key = candidate.rate_limit_key
                              AND active.status IN ('Scheduled', 'Running')
                          ) >= rl.max_inflight
                    )
                  )
                ORDER BY candidate.priority DESC, candidate.created_at ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            )
            RETURNING *
            "#,
        )
        .bind(runtime_pack_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        let row = match row {
            Some(r) => r,
            None => {
                tx.commit()
                    .await
                    .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
                return Ok(None);
            }
        };

        let task_id: Uuid = row.get("id");
        let lease_id = Uuid::new_v4();

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
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            artifact_id: row.get("artifact_id"),
            runtime_pack_id: row.get("runtime_pack_id"),
            status,
            priority: row.get("priority"),
            rate_limit_key: row.get("rate_limit_key"),
            payload_ref: row.get("payload_ref"),
            timeout_seconds: row.get("timeout_seconds"),
            max_attempts: row.get("max_attempts"),
            current_attempt: row.get("current_attempt"),
            idempotency_key: row.get("idempotency_key"),
            payload_size_bytes: row.get("payload_size_bytes"),
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
        logs_ref: Option<String>,
        result_ref: Option<String>,
        last_error_message: Option<String>,
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

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        let lease_exists = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM task_leases
                WHERE id = $1
                  AND task_id = $2
                  AND executor_id = $3
                  AND expires_at > NOW()
            )
            "#,
            lease_id,
            id,
            executor_id
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?
        .unwrap_or(false);

        if !lease_exists {
            return Err(RepositoryError::Conflict(
                "no active lease matches this completion".to_string(),
            ));
        }

        match status {
            TaskStatus::Running => {
                let result = sqlx::query!(
                    r#"
                    UPDATE tasks
                    SET status = $1, current_attempt = current_attempt + 1, updated_at = NOW()
                    WHERE id = $2
                      AND status = 'Scheduled'
                    "#,
                    status_str,
                    id
                )
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

                if result.rows_affected() == 0 {
                    let status_row = sqlx::query!("SELECT status FROM tasks WHERE id = $1", id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

                    let already_running = status_row
                        .as_ref()
                        .map(|row| row.status == "Running")
                        .unwrap_or(false);

                    if !already_running {
                        return Err(RepositoryError::Conflict(
                            "task is not schedulable for running".to_string(),
                        ));
                    }
                }

                sqlx::query!(
                    r#"
                    INSERT INTO task_attempts (id, task_id, executor_id, lease_id)
                    SELECT $1, $2, $3, $4
                    WHERE NOT EXISTS (
                        SELECT 1 FROM task_attempts WHERE lease_id = $4
                    )
                    "#,
                    Uuid::new_v4(),
                    id,
                    executor_id,
                    lease_id
                )
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
            }
            TaskStatus::Succeeded
            | TaskStatus::Failed
            | TaskStatus::Cancelled
            | TaskStatus::TimedOut
            | TaskStatus::DeadLetter => {
                let result = sqlx::query!(
                    r#"
                    UPDATE tasks
                    SET status = $1, result_ref = COALESCE($3, result_ref), last_error_message = $4, updated_at = NOW()
                    WHERE id = $2
                    "#,
                    status_str,
                    id,
                    result_ref,
                    last_error_message
                )
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

                if result.rows_affected() == 0 {
                    return Err(RepositoryError::NotFound);
                }

                sqlx::query!(
                    r#"
                    UPDATE task_attempts
                    SET finished_at = NOW(), exit_reason = $1, logs_ref = COALESCE($3, logs_ref), result_ref = COALESCE($4, result_ref)
                    WHERE lease_id = $2
                      AND finished_at IS NULL
                    "#,
                    status_str,
                    lease_id,
                    logs_ref,
                    result_ref
                )
                .execute(&mut *tx)
                .await
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

                sqlx::query!("DELETE FROM task_leases WHERE id = $1", lease_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
            }
            TaskStatus::Queued | TaskStatus::Scheduled => {
                return Err(RepositoryError::Conflict(
                    "unsupported external status transition".to_string(),
                ));
            }
        }

        tx.commit()
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn requeue_expired_leases(&self) -> Result<u64, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        let expired = sqlx::query!(
            r#"
            SELECT tl.id, tl.task_id, t.current_attempt, t.max_attempts
            FROM task_leases tl
            JOIN tasks t ON t.id = tl.task_id
            WHERE tl.expires_at <= NOW()
              AND t.status IN ('Scheduled', 'Running')
            "#
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if expired.is_empty() {
            tx.commit()
                .await
                .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
            return Ok(0);
        }

        for lease in &expired {
            let next_status = if lease.current_attempt >= lease.max_attempts {
                "DeadLetter"
            } else {
                "Queued"
            };

            sqlx::query!(
                r#"
                UPDATE tasks
                SET status = $1, updated_at = NOW()
                WHERE id = $2
                "#,
                next_status,
                lease.task_id
            )
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

            sqlx::query!(
                r#"
                UPDATE task_attempts
                SET finished_at = NOW(), exit_reason = 'LeaseExpired'
                WHERE lease_id = $1
                  AND finished_at IS NULL
                "#,
                lease.id
            )
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
        }

        let lease_ids: Vec<Uuid> = expired.iter().map(|lease| lease.id).collect();
        sqlx::query!("DELETE FROM task_leases WHERE id = ANY($1)", &lease_ids)
            .execute(&mut *tx)
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(expired.len() as u64)
    }

    async fn cancel_task(&self, id: Uuid) -> Result<(), RepositoryError> {
        let result = sqlx::query!(
            r#"
            UPDATE tasks
            SET status = 'Cancelled', cancel_requested_at = NOW(), updated_at = NOW()
            WHERE id = $1
              AND status NOT IN ('Succeeded', 'Failed', 'Cancelled', 'TimedOut', 'DeadLetter')
            "#,
            id
        )
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if result.rows_affected() > 0 {
            return Ok(());
        }

        let exists = sqlx::query_scalar!("SELECT EXISTS (SELECT 1 FROM tasks WHERE id = $1)", id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?
            .unwrap_or(false);

        if exists {
            Err(RepositoryError::Conflict(
                "task is already terminal".to_string(),
            ))
        } else {
            Err(RepositoryError::NotFound)
        }
    }
}
