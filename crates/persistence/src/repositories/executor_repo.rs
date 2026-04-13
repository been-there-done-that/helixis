use application::ports::repositories::{ExecutorRepository, RepositoryError};
use async_trait::async_trait;
use domain::Executor;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresExecutorRepository {
    pool: PgPool,
}

impl PostgresExecutorRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ExecutorRepository for PostgresExecutorRepository {
    async fn upsert_executor(&self, executor: &Executor) -> Result<(), RepositoryError> {
        let json_caps = serde_json::json!(executor.capabilities);
        sqlx::query!(
            r#"
            INSERT INTO executors (id, session_id, capabilities_json)
            VALUES ($1, $2, $3)
            ON CONFLICT (id) DO UPDATE SET session_id = EXCLUDED.session_id, last_heartbeat_at = NOW()
            "#,
            executor.id, executor.session_id, json_caps
        ).execute(&self.pool).await.map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    async fn record_heartbeat(&self, id: Uuid) -> Result<(), RepositoryError> {
        let result = sqlx::query!(
            "UPDATE executors SET last_heartbeat_at = NOW() WHERE id = $1",
            id
        )
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }
}
