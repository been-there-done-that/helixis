use crate::artifact_client::ArtifactDownloader;
use crate::client::CplaneClient;
use crate::output_store::OutputUploader;
use crate::runtime::RuntimeAdapter;
use domain::{Task, TaskLease, TaskStatus};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{Duration, interval, sleep};

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
        environment: BTreeMap<String, String>,
        payload: Option<Value>,
    ) -> Result<ExecutionOutcome, String>;
}

pub struct ProcessSandbox {
    pub downloader: ArtifactDownloader,
    pub output_uploader: OutputUploader,
    pub runtime: Box<dyn RuntimeAdapter>,
}

#[async_trait::async_trait]
impl TaskSandbox for ProcessSandbox {
    async fn execute(
        &self,
        task: &Task,
        lease: &TaskLease,
        client: Arc<CplaneClient>,
        environment: BTreeMap<String, String>,
        payload: Option<Value>,
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

        let run_dir = Path::new("/tmp/helixis/tasks").join(task.id.to_string());
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
        let payload_path = materialize_payload(&run_dir, payload).await?;

        let mut command: Command = self.runtime.build_command(&run_dir);
        command
            .current_dir(&run_dir)
            .env("TASK_WORKSPACE", &run_dir)
            .envs(environment)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(payload_path) = &payload_path {
            command.env("TASK_PAYLOAD_PATH", payload_path);
        }

        let mut child = command
            .spawn()
            .map_err(|e| format!("Failed to spawn process: {}", e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to capture stdout pipe".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Failed to capture stderr pipe".to_string())?;

        let stdout_task = tokio::spawn(stream_and_capture(
            stdout,
            "stdout",
            Arc::clone(&client),
            task.id,
            lease.id,
        ));
        let stderr_task = tokio::spawn(stream_and_capture(
            stderr,
            "stderr",
            Arc::clone(&client),
            task.id,
            lease.id,
        ));

        let mut cancel_ticker = interval(Duration::from_secs(1));
        let mut cancelled = false;
        let mut timed_out = false;
        let timeout = sleep(Duration::from_secs(task.timeout_seconds.max(1) as u64));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                status = child.wait() => {
                    let status = status.map_err(|e| format!("Failed while waiting for process: {}", e))?;
                    let stdout_bytes = stdout_task
                        .await
                        .map_err(|e| e.to_string())??;
                    let stderr_bytes = stderr_task
                        .await
                        .map_err(|e| e.to_string())??;

                    let outcome = build_outcome(
                        &self.output_uploader,
                        task,
                        lease,
                        status.code(),
                        cancelled,
                        timed_out,
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
                    if !cancelled && !timed_out {
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
                _ = &mut timeout, if !cancelled && !timed_out => {
                    tracing::warn!(
                        "Task {} exceeded timeout of {} seconds",
                        task.id,
                        task.timeout_seconds
                    );
                    timed_out = true;
                    child.kill()
                        .await
                        .map_err(|e| format!("Failed to kill timed-out process: {}", e))?;
                }
            }
        }
    }
}

async fn materialize_payload(
    run_dir: &Path,
    payload: Option<Value>,
) -> Result<Option<PathBuf>, String> {
    let Some(payload) = payload else {
        return Ok(None);
    };

    let payload_path = run_dir.join("payload.json");
    tokio::fs::write(&payload_path, payload.to_string())
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(payload_path))
}

async fn build_outcome(
    output_uploader: &OutputUploader,
    task: &Task,
    lease: &TaskLease,
    exit_code: Option<i32>,
    cancelled: bool,
    timed_out: bool,
    stdout_bytes: Vec<u8>,
    stderr_bytes: Vec<u8>,
) -> Result<ExecutionOutcome, String> {
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);
    let combined_logs = format!("--- stdout ---\n{}\n--- stderr ---\n{}\n", stdout, stderr);

    let status = if timed_out {
        TaskStatus::TimedOut
    } else if cancelled {
        TaskStatus::Cancelled
    } else if exit_code == Some(0) {
        TaskStatus::Succeeded
    } else {
        TaskStatus::Failed
    };

    let last_error_message = match status {
        TaskStatus::Failed => Some(format!(
            "Process exited with code {}{}",
            exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        )),
        TaskStatus::TimedOut => Some(format!(
            "Execution exceeded timeout of {} seconds",
            task.timeout_seconds
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
        "timed_out": timed_out,
    });

    let (logs_ref, result_ref) = output_uploader
        .upload_outputs(task.id, lease.id, combined_logs, result_json.to_string())
        .await
        .map_err(|e| e.to_string())?;

    Ok(ExecutionOutcome {
        status,
        logs_ref: Some(logs_ref),
        result_ref: Some(result_ref),
        last_error_message,
    })
}

async fn stream_and_capture<R>(
    mut reader: R,
    stream_name: &'static str,
    client: Arc<CplaneClient>,
    task_id: uuid::Uuid,
    lease_id: uuid::Uuid,
) -> Result<Vec<u8>, String>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut all = Vec::new();
    let mut buf = [0u8; 4096];

    loop {
        let read = reader.read(&mut buf).await.map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }

        let chunk = &buf[..read];
        all.extend_from_slice(chunk);

        let chunk_text = String::from_utf8_lossy(chunk).to_string();
        if let Err(err) = client
            .append_log_chunk(task_id, lease_id, stream_name, chunk_text)
            .await
        {
            tracing::warn!("Failed to append live log chunk: {}", err);
        }
    }

    Ok(all)
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
