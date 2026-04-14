use async_trait::async_trait;
use domain::{Artifact, Executor, Task, TaskLease, TaskOutputs, TaskStatus};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Record not found")]
    NotFound,
    #[error("Conflict: {0}")]
    Conflict(String),
}

#[async_trait]
pub trait TaskRepository: Send + Sync {
    async fn get_task(&self, id: Uuid) -> Result<Option<Task>, RepositoryError>;
    async fn get_task_outputs(&self, id: Uuid) -> Result<Option<TaskOutputs>, RepositoryError>;

    async fn insert_task(&self, task: &Task) -> Result<(), RepositoryError>;

    /// Polling queue items natively
    async fn poll_and_lease(
        &self,
        runtime_pack_id: &str,
        executor_id: Uuid,
        lease_duration_sec: i32,
    ) -> Result<Option<(Task, TaskLease)>, RepositoryError>;

    async fn update_task_status(
        &self,
        id: Uuid,
        lease_id: Uuid,
        executor_id: Uuid,
        status: TaskStatus,
        logs_ref: Option<String>,
        result_ref: Option<String>,
        last_error_message: Option<String>,
    ) -> Result<(), RepositoryError>;

    async fn requeue_expired_leases(&self) -> Result<u64, RepositoryError>;

    async fn cancel_task(&self, id: Uuid) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait ExecutorRepository: Send + Sync {
    async fn upsert_executor(&self, executor: &Executor) -> Result<(), RepositoryError>;
    async fn record_heartbeat(&self, id: Uuid) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait ArtifactRepository: Send + Sync {
    async fn insert_artifact(&self, artifact: &Artifact) -> Result<(), RepositoryError>;
    async fn get_artifact(&self, id: Uuid) -> Result<Option<Artifact>, RepositoryError>;
}

#[async_trait]
pub trait RateLimitRepository: Send + Sync {
    async fn put_rate_limit(
        &self,
        tenant_id: Uuid,
        key: &str,
        max_inflight: i32,
    ) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait SecretRepository: Send + Sync {
    async fn put_secret(
        &self,
        tenant_id: Uuid,
        key: &str,
        value: &str,
    ) -> Result<(), RepositoryError>;

    async fn get_tenant_secrets(
        &self,
        tenant_id: Uuid,
    ) -> Result<BTreeMap<String, String>, RepositoryError>;
}
