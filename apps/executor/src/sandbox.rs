use crate::artifact_client::ArtifactDownloader;
use crate::client::CplaneClient;
use domain::{Task, TaskLease, TaskStatus};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{Duration, interval};

pub struct ExecutionOutcome {
    pub status: TaskStatus,
    pub logs_ref: Option<String>,
    pub result_ref: Option<String>,
    pub last_error_message: Option<String>,
}

#[async_trait::async_trait]
pub trait TaskSandbox {
    async fn execute(
        &self,
        task: &Task,
        lease: &TaskLease,
        client: Arc<CplaneClient>,
    ) -> Result<ExecutionOutcome, String>;
}

pub struct ProcessSandbox {
    pub downloader: ArtifactDownloader,
    pub command: String,
    pub entrypoint: String,
}

#[async_trait::async_trait]
impl TaskSandbox for ProcessSandbox {
    async fn execute(
        &self,
        task: &Task,
        lease: &TaskLease,
        client: Arc<CplaneClient>,
    ) -> Result<ExecutionOutcome, String> {
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

        let artifact_dir = Path::new("/tmp/helixis/task-artifacts")
            .join(task.id.to_string())
            .join(lease.id.to_string());
        tokio::fs::create_dir_all(&artifact_dir)
            .await
            .map_err(|e| e.to_string())?;

        tracing::info!(
            "ProcessSandbox: Executing task [{}] via command spawn...",
            task.id
        );

        let entrypoint = run_dir.join(&self.entrypoint);

        let mut child = Command::new(&self.command)
            .arg(entrypoint)
            .current_dir(&run_dir)
            .env("TASK_WORKSPACE", &run_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn process: {}", e))?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to capture stdout pipe".to_string())?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Failed to capture stderr pipe".to_string())?;

        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            stdout.read_to_end(&mut buf).await.map(|_| buf)
        });
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            stderr.read_to_end(&mut buf).await.map(|_| buf)
        });

        let mut cancel_ticker = interval(Duration::from_secs(1));
        let mut cancelled = false;

        loop {
            tokio::select! {
                status = child.wait() => {
                    let status = status.map_err(|e| format!("Failed while waiting for process: {}", e))?;
                    let stdout_bytes = stdout_task
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?;
                    let stderr_bytes = stderr_task
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?;

                    let outcome = build_outcome(
                        task,
                        lease,
                        &artifact_dir,
                        status.code(),
                        cancelled,
                        stdout_bytes,
                        stderr_bytes,
                    )
                    .await?;

                    if let Err(err) = tokio::fs::remove_dir_all(&run_dir).await {
                        tracing::warn!("Failed to clean up task directory {:?}: {}", run_dir, err);
                    }

                    return Ok(outcome);
                }
                _ = cancel_ticker.tick() => {
                    match client.get_task_status(task.id).await {
                        Ok(TaskStatus::Cancelled) => {
                            tracing::info!("Cancellation requested for task {}. Stopping process.", task.id);
                            cancelled = true;
                            child.kill().await.map_err(|e| format!("Failed to kill process: {}", e))?;
                        }
                        Ok(_) => {}
                        Err(err) => {
                            tracing::warn!("Failed to check task status for cancellation: {}", err);
                        }
                    }
                }
            }
        }
    }
}

async fn build_outcome(
    task: &Task,
    lease: &TaskLease,
    artifact_dir: &Path,
    exit_code: Option<i32>,
    cancelled: bool,
    stdout_bytes: Vec<u8>,
    stderr_bytes: Vec<u8>,
) -> Result<ExecutionOutcome, String> {
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);
    let combined_logs = format!("--- stdout ---\n{}\n--- stderr ---\n{}\n", stdout, stderr);

    let logs_path = artifact_dir.join("logs.txt");
    tokio::fs::write(&logs_path, combined_logs)
        .await
        .map_err(|e| e.to_string())?;

    let status = if cancelled {
        TaskStatus::Cancelled
    } else if exit_code == Some(0) {
        TaskStatus::Succeeded
    } else {
        TaskStatus::Failed
    };

    let last_error_message = match status {
        TaskStatus::Failed => Some(format!(
            "Process exited with code {}{}",
            exit_code.map(|code| code.to_string()).unwrap_or_else(|| "unknown".to_string()),
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        )),
        TaskStatus::Cancelled => Some("Execution cancelled by control plane".to_string()),
        _ => None,
    };

    let result_json = serde_json::json!({
        "task_id": task.id,
        "lease_id": lease.id,
        "status": format!("{:?}", status),
        "exit_code": exit_code,
        "cancelled": cancelled,
    });

    let result_path = artifact_dir.join("result.json");
    tokio::fs::write(&result_path, result_json.to_string())
        .await
        .map_err(|e| e.to_string())?;

    Ok(ExecutionOutcome {
        status,
        logs_ref: Some(logs_path.to_string_lossy().into_owned()),
        result_ref: Some(result_path.to_string_lossy().into_owned()),
        last_error_message,
    })
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
