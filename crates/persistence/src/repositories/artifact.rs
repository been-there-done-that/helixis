use application::ports::repositories::{ArtifactRepository, RepositoryError};
use async_trait::async_trait;
use domain::{Artifact, ArtifactStatus, ArtifactUploadSession, ArtifactUploadStatus};
use sqlx::{PgPool, Row};
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
    async fn create_artifact_upload(
        &self,
        artifact: &Artifact,
    ) -> Result<ArtifactUploadSession, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        let result = sqlx::query(
            r#"
            INSERT INTO artifacts (id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes, status, object_key)
            VALUES ($1, $2, $3, $4, $5, $6, 'PendingUpload', NULL)
            ON CONFLICT (digest) DO NOTHING
            "#,
        )
        .bind(artifact.id)
        .bind(artifact.tenant_id)
        .bind(&artifact.digest)
        .bind(&artifact.runtime_pack_id)
        .bind(&artifact.entrypoint)
        .bind(artifact.size_bytes)
        .execute(&mut *tx)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::Conflict(
                "artifact digest already exists".to_string(),
            ));
        }

        let session = ArtifactUploadSession {
            id: Uuid::new_v4(),
            artifact_id: artifact.id,
            status: ArtifactUploadStatus::Pending,
        };

        sqlx::query(
            r#"
            INSERT INTO artifact_upload_sessions (id, artifact_id, status)
            VALUES ($1, $2, 'Pending')
            "#,
        )
        .bind(session.id)
        .bind(session.artifact_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        tx.commit()
            .await
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        Ok(session)
    }

    async fn get_artifact(&self, id: Uuid) -> Result<Option<Artifact>, RepositoryError> {
        let row = sqlx::query(
            r#"
            SELECT id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes, status, object_key
            FROM artifacts
            WHERE id = $1
            "#,
        )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        match row {
            Some(artifact) => Ok(Some(Artifact {
                id: artifact.get("id"),
                tenant_id: artifact.get("tenant_id"),
                digest: artifact.get("digest"),
                runtime_pack_id: artifact.get("runtime_pack_id"),
                entrypoint: artifact.get("entrypoint"),
                size_bytes: artifact.get("size_bytes"),
                status: parse_artifact_status(artifact.get::<&str, _>("status"))?,
                object_key: artifact.get("object_key"),
            })),
            None => Ok(None),
        }
    }

    async fn get_upload_session(
        &self,
        id: Uuid,
    ) -> Result<Option<ArtifactUploadSession>, RepositoryError> {
        let row = sqlx::query(
            "SELECT id, artifact_id, status FROM artifact_upload_sessions WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        Ok(row.map(|session| ArtifactUploadSession {
            id: session.get("id"),
            artifact_id: session.get("artifact_id"),
            status: parse_artifact_upload_status(session.get::<&str, _>("status")),
        }))
    }

    async fn complete_artifact_upload(
        &self,
        session_id: Uuid,
        object_key: &str,
    ) -> Result<Artifact, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        let session = sqlx::query(
            "SELECT artifact_id, status FROM artifact_upload_sessions WHERE id = $1 FOR UPDATE",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?
        .ok_or(RepositoryError::NotFound)?;

        if session.get::<&str, _>("status") != "Pending" {
            return Err(RepositoryError::Conflict(
                "artifact upload session is no longer pending".to_string(),
            ));
        }

        let row = sqlx::query(
            r#"
            UPDATE artifacts
            SET status = 'Ready', object_key = $2
            WHERE id = $1
            RETURNING id, tenant_id, digest, runtime_pack_id, entrypoint, size_bytes, status, object_key
            "#,
        )
        .bind(session.get::<Uuid, _>("artifact_id"))
        .bind(object_key)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        sqlx::query(
            r#"
            UPDATE artifact_upload_sessions
            SET status = 'Completed', completed_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(session_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        tx.commit()
            .await
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        Ok(Artifact {
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            digest: row.get("digest"),
            runtime_pack_id: row.get("runtime_pack_id"),
            entrypoint: row.get("entrypoint"),
            size_bytes: row.get("size_bytes"),
            status: parse_artifact_status(row.get::<&str, _>("status"))?,
            object_key: row.get("object_key"),
        })
    }
}

fn parse_artifact_status(value: &str) -> Result<ArtifactStatus, RepositoryError> {
    match value {
        "PendingUpload" => Ok(ArtifactStatus::PendingUpload),
        "Ready" => Ok(ArtifactStatus::Ready),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown artifact status {other}"
        ))),
    }
}

fn parse_artifact_upload_status(value: &str) -> ArtifactUploadStatus {
    match value {
        "Completed" => ArtifactUploadStatus::Completed,
        _ => ArtifactUploadStatus::Pending,
    }
}
