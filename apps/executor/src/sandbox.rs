use crate::artifact_client::ArtifactDownloader;
use domain::Task;
use std::path::Path;
use tokio::process::Command;

#[async_trait::async_trait]
pub trait TaskSandbox {
    async fn execute(&self, task: &Task) -> Result<(), String>;
}

pub struct ProcessSandbox {
    pub downloader: ArtifactDownloader,
    pub command: String,
    pub entrypoint: String,
}

#[async_trait::async_trait]
impl TaskSandbox for ProcessSandbox {
    async fn execute(&self, task: &Task) -> Result<(), String> {
        tracing::info!(
            "ProcessSandbox: Checking S3 artifact cache for [{}]...",
            task.artifact_id
        );

        let cache_dir = self
            .downloader
            .ensure_artifact_cached(task.artifact_id)
            .await
            .map_err(|e| format!("Failed to fetch artifact: {}", e))?;

        // ephemeral run directory
        let run_dir = Path::new("/tmp/helixis/tasks").join(format!("{}", task.id));
        tokio::fs::create_dir_all(&run_dir)
            .await
            .map_err(|e| e.to_string())?;

        tracing::info!(
            "ProcessSandbox: Executing task [{}] via command spawn...",
            task.id
        );

        // Spawn the binary/script using tokio Command so we don't block
        // For security, true sandboxes use unshare/cgroups. Here we just run natively.
        // It's assumed the entrypoint is a valid script in the unpacked cache
        let entrypoint = cache_dir.join(&self.entrypoint);

        let output = Command::new(&self.command)
            .arg(entrypoint)
            .current_dir(&cache_dir) // Execute inside the cache root (or better, copy to run_dir for safety)
            .env("TASK_WORKSPACE", &run_dir)
            .output()
            .await
            .map_err(|e| format!("Failed to spawn process: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!("Task {} failed with stderr: {}", task.id, stderr);
            return Err(format!(
                "Process crashed with code {}: {}",
                output.status, stderr
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        tracing::info!(
            "ProcessSandbox: Task [{}] succeeded! Output: \n{}",
            task.id,
            stdout
        );

        // Cleanup ephemeral run_dir
        let _ = tokio::fs::remove_dir_all(&run_dir).await;

        Ok(())
    }
}
