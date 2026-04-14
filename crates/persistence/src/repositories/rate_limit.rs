use application::ports::repositories::{RateLimitRepository, RepositoryError};
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresRateLimitRepository {
    pool: PgPool,
}

impl PostgresRateLimitRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RateLimitRepository for PostgresRateLimitRepository {
    async fn put_rate_limit(
        &self,
        tenant_id: Uuid,
        key: &str,
        max_inflight: i32,
    ) -> Result<(), RepositoryError> {
        if max_inflight <= 0 {
            return Err(RepositoryError::Conflict(
                "max_inflight must be greater than zero".to_string(),
            ));
        }

        sqlx::query(
            r#"
            INSERT INTO rate_limits (tenant_id, rate_limit_key, max_inflight, updated_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (tenant_id, rate_limit_key)
            DO UPDATE
            SET max_inflight = EXCLUDED.max_inflight,
                updated_at = NOW()
            "#,
        )
        .bind(tenant_id)
        .bind(key)
        .bind(max_inflight)
        .execute(&self.pool)
        .await
        .map_err(|e| RepositoryError::DatabaseError(e.to_string()))?;

        Ok(())
    }
}
