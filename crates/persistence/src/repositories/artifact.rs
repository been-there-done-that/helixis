use application::ports::repositories::{ArtifactRepository, RepositoryError};
use async_trait::async_trait;
use domain::Artifact;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PostgresArtifactRepository {
    pool: PgPool,
}

impl PostgresArtifactRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ArtifactRepository for PostgresArtifactRepository {
    async fn insert_artifact(&self, artifact: &Artifact) -> Result<(), RepositoryError> {
        let result = sqlx::query!(
            r#"
            INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (digest) DO NOTHING
            "#,
            artifact.id,
            artifact.tenant_id,
            artifact.digest,
            artifact.runtime_pack_id,
            artifact.entrypoint,
            artifact.size_bytes
        )
        .execute(&self.pool)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::Conflict(
                "artifact digest already exists".to_string(),
            ));
        }

        Ok(())
    }

    async fn get_artifact(&self, id: Uuid) -> Result<Option<Artifact>, RepositoryError> {
        let row = sqlx::query!("SELECT * FROM artifacts WHERE id = $1", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        Ok(row.map(|artifact| Artifact {
            id: artifact.id,
            tenant_id: artifact.tenant_id,
            digest: artifact.digest,
            runtime_pack_id: artifact.runtime_pack_id,
            entrypoint: artifact.entrypoint,
            size_bytes: artifact.size_bytes,
        }))
    }
}
