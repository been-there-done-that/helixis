use crate::artifact_client::ArtifactDownloader;
use domain::Task;
use std::path::{Path, PathBuf};
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

        let run_dir = Path::new("/tmp/helixis/tasks").join(format!("{}", task.id));
        if tokio::fs::try_exists(&run_dir)
            .await
            .map_err(|e| e.to_string())?
        {
            tokio::fs::remove_dir_all(&run_dir)
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::create_dir_all(&run_dir)
            .await
            .map_err(|e| e.to_string())?;
        copy_dir_recursive(&cache_dir, &run_dir).await?;

        tracing::info!(
            "ProcessSandbox: Executing task [{}] via command spawn...",
            task.id
        );

        // Spawn the binary/script using tokio Command so we don't block
        // For security, true sandboxes use unshare/cgroups. Here we just run natively.
        // It's assumed the entrypoint is a valid script in the unpacked cache
        let entrypoint = run_dir.join(&self.entrypoint);

        let execution_result = Command::new(&self.command)
            .arg(entrypoint)
            .current_dir(&run_dir)
            .env("TASK_WORKSPACE", &run_dir)
            .output()
            .await
            .map_err(|e| format!("Failed to spawn process: {}", e));

        let result = match execution_result {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                tracing::info!(
                    "ProcessSandbox: Task [{}] succeeded! Output: \n{}",
                    task.id,
                    stdout
                );
                Ok(())
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::error!("Task {} failed with stderr: {}", task.id, stderr);
                Err(format!(
                    "Process crashed with code {}: {}",
                    output.status, stderr
                ))
            }
            Err(err) => Err(err),
        };

        if let Err(err) = tokio::fs::remove_dir_all(&run_dir).await {
            tracing::warn!("Failed to clean up task directory {:?}: {}", run_dir, err);
        }

        result
    }
}

async fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    let source = source.to_path_buf();
    let destination = destination.to_path_buf();

    tokio::task::spawn_blocking(move || copy_dir_recursive_blocking(&source, &destination))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

fn copy_dir_recursive_blocking(source: &Path, destination: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let target_path: PathBuf = destination.join(entry.file_name());

        if entry_type.is_dir() {
            std::fs::create_dir_all(&target_path)?;
            copy_dir_recursive_blocking(&entry.path(), &target_path)?;
        } else if entry_type.is_file() {
            std::fs::copy(entry.path(), target_path)?;
        }
    }

    Ok(())
}
