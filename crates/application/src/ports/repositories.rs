use async_trait::async_trait;
use domain::{Executor, Task, TaskLease, TaskStatus};
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
    ) -> Result<(), RepositoryError>;
}

#[async_trait]
pub trait ExecutorRepository: Send + Sync {
    async fn upsert_executor(&self, executor: &Executor) -> Result<(), RepositoryError>;
    async fn record_heartbeat(&self, id: Uuid) -> Result<(), RepositoryError>;
}
