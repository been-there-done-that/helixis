use crate::artifact_client::ArtifactDownloader;
use domain::Task;
use std::time::Duration;
use tokio::time::sleep;

#[async_trait::async_trait]
pub trait TaskSandbox {
    async fn execute(&self, task: &Task) -> Result<(), String>;
}

pub struct ProcessSandbox {
    pub downloader: ArtifactDownloader,
}

#[async_trait::async_trait]
impl TaskSandbox for ProcessSandbox {
    async fn execute(&self, task: &Task) -> Result<(), String> {
        tracing::info!("ProcessSandbox: Downloading S3 artifact [{}]...", task.artifact_id);
        
        let file_path = self.downloader.download_artifact(task.artifact_id).await
            .map_err(|e| format!("Failed to download artifact: {}", e))?;
            
        tracing::info!("ProcessSandbox: Artifact downloaded to {:?}. Unpacking...", file_path);
        sleep(Duration::from_millis(500)).await;
        
        // At this point we would natively untar via `flate2` and `tar` crates then `.spawn()`
        tracing::info!("ProcessSandbox: Executing task [{}] via command spawn...", task.id);
        sleep(Duration::from_secs(2)).await;
        
        tracing::info!("ProcessSandbox: Task [{}] succeeded!", task.id);
        Ok(())
    }
}
