use application::ports::repositories::{PayloadRepository, RepositoryError};
use async_trait::async_trait;
use domain::{PayloadObject, PayloadStatus, PayloadUploadSession, PayloadUploadStatus};
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub struct PostgresPayloadRepository {
    pool: PgPool,
}

impl PostgresPayloadRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PayloadRepository for PostgresPayloadRepository {
    async fn create_payload_upload(
        &self,
        payload: &PayloadObject,
    ) -> Result<PayloadUploadSession, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        let result = sqlx::query(
            r#"
            INSERT INTO payloads (id, tenant_id, digest, size_bytes, status, object_key)
            VALUES ($1, $2, $3, $4, 'PendingUpload', NULL)
            ON CONFLICT (digest) DO NOTHING
            "#,
        )
        .bind(payload.id)
        .bind(payload.tenant_id)
        .bind(&payload.digest)
        .bind(payload.size_bytes)
        .execute(&mut *tx)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(RepositoryError::Conflict(
                "payload digest already exists".to_string(),
            ));
        }

        let session = PayloadUploadSession {
            id: Uuid::new_v4(),
            payload_id: payload.id,
            status: PayloadUploadStatus::Pending,
        };

        sqlx::query(
            r#"
            INSERT INTO payload_upload_sessions (id, payload_id, status)
            VALUES ($1, $2, 'Pending')
            "#,
        )
        .bind(session.id)
        .bind(session.payload_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        tx.commit()
            .await
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        Ok(session)
    }

    async fn get_payload(&self, id: Uuid) -> Result<Option<PayloadObject>, RepositoryError> {
        let row = sqlx::query(
            r#"
            SELECT id, tenant_id, digest, size_bytes, status, object_key
            FROM payloads
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        match row {
            Some(payload) => Ok(Some(PayloadObject {
                id: payload.get("id"),
                tenant_id: payload.get("tenant_id"),
                digest: payload.get("digest"),
                size_bytes: payload.get("size_bytes"),
                status: parse_payload_status(payload.get::<&str, _>("status"))?,
                object_key: payload.get("object_key"),
            })),
            None => Ok(None),
        }
    }

    async fn get_upload_session(
        &self,
        id: Uuid,
    ) -> Result<Option<PayloadUploadSession>, RepositoryError> {
        let row =
            sqlx::query("SELECT id, payload_id, status FROM payload_upload_sessions WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        Ok(row.map(|session| PayloadUploadSession {
            id: session.get("id"),
            payload_id: session.get("payload_id"),
            status: parse_payload_upload_status(session.get::<&str, _>("status")),
        }))
    }

    async fn complete_payload_upload(
        &self,
        session_id: Uuid,
        object_key: &str,
    ) -> Result<PayloadObject, RepositoryError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        let session = sqlx::query(
            "SELECT payload_id, status FROM payload_upload_sessions WHERE id = $1 FOR UPDATE",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?
        .ok_or(RepositoryError::NotFound)?;

        if session.get::<&str, _>("status") != "Pending" {
            return Err(RepositoryError::Conflict(
                "payload upload session is no longer pending".to_string(),
            ));
        }

        let row = sqlx::query(
            r#"
            UPDATE payloads
            SET status = 'Ready', object_key = $2
            WHERE id = $1
            RETURNING id, tenant_id, digest, size_bytes, status, object_key
            "#,
        )
        .bind(session.get::<Uuid, _>("payload_id"))
        .bind(object_key)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| RepositoryError::DatabaseError(err.to_string()))?;

        sqlx::query(
            r#"
            UPDATE payload_upload_sessions
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

        Ok(PayloadObject {
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            digest: row.get("digest"),
            size_bytes: row.get("size_bytes"),
            status: parse_payload_status(row.get::<&str, _>("status"))?,
            object_key: row.get("object_key"),
        })
    }
}

fn parse_payload_status(value: &str) -> Result<PayloadStatus, RepositoryError> {
    match value {
        "PendingUpload" => Ok(PayloadStatus::PendingUpload),
        "Ready" => Ok(PayloadStatus::Ready),
        other => Err(RepositoryError::DatabaseError(format!(
            "unknown payload status {other}"
        ))),
    }
}

fn parse_payload_upload_status(value: &str) -> PayloadUploadStatus {
    match value {
        "Completed" => PayloadUploadStatus::Completed,
        _ => PayloadUploadStatus::Pending,
    }
}
