use domain::Task;
use std::time::Duration;
use tokio::time::sleep;

#[async_trait::async_trait]
pub trait TaskSandbox {
    async fn execute(&self, task: &Task) -> Result<(), String>;
}

pub struct MockSandbox;

#[async_trait::async_trait]
impl TaskSandbox for MockSandbox {
    async fn execute(&self, task: &Task) -> Result<(), String> {
        tracing::info!("MockSandbox: Downloading artifact S3: [{}] for task [{}]...", task.artifact_id, task.id);
        sleep(Duration::from_millis(500)).await;
        
        tracing::info!("MockSandbox: Executing task [{}]...", task.id);
        sleep(Duration::from_secs(2)).await;
        
        tracing::info!("MockSandbox: Task [{}] succeeded!", task.id);
        Ok(())
    }
}
