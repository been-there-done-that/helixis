mod artifact_client;
mod client;
mod sandbox;

use artifact_client::ArtifactDownloader;
use client::CplaneClient;
use sandbox::{ProcessSandbox, TaskSandbox};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    telemetry::init_logging();

    let executor_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    tracing::info!("Starting Helixis Executor Agent [{executor_id}]...");

    let client = Arc::new(
        CplaneClient::new("http://127.0.0.1:3000", executor_id, session_id)
            .expect("Failed to create unified Control Plane internal client"),
    );

    let downloader = ArtifactDownloader::new(
        "http://localhost:9000",
        "minioadmin",
        "minioadmin",
        "artifacts",
    );

    let command = std::env::var("EXECUTOR_COMMAND").unwrap_or_else(|_| "python3".to_string());
    let entrypoint = std::env::var("EXECUTOR_ENTRYPOINT").unwrap_or_else(|_| "main.py".to_string());
    let sandbox = ProcessSandbox { downloader, command, entrypoint };
    
    let env_pack_id =
        std::env::var("RUNTIME_PACK_ID").unwrap_or_else(|_| "demo-python-pack".to_string());
    let runtime_pack_id = env_pack_id.clone();

    client
        .register_executor(vec![runtime_pack_id.clone()])
        .await
        .expect("Failed to register executor");

    let heartbeat_client = Arc::clone(&client);
    let heartbeat_executor_id = executor_id;
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(15)).await;
            if let Err(err) = heartbeat_client.send_heartbeat().await {
                tracing::warn!(executor_id = %heartbeat_executor_id, "Executor heartbeat failed: {}", err);
            }
        }
    });

    tracing::info!(
        "Polling for tasks matching runtime_pack: {}",
        runtime_pack_id
    );

    let mut consecutive_empty_polls = 0u32;

    loop {
        let span = tracing::info_span!("poll_loop", runtime_pack_id = %runtime_pack_id);
        let _enter = span.enter();

        tracing::debug!("Polling queue...");

        match client.poll_task(&runtime_pack_id, 120).await {
            Ok(Some((task, lease))) => {
                consecutive_empty_polls = 0;
                tracing::info!(task_id = %task.id, "Acquired task! Executing now...");

                if let Err(err) = client
                    .report_status(task.id, lease.id, "Running", None, None, None)
                    .await
                {
                    tracing::error!(task_id = %task.id, "Failed to mark task as running: {}", err);
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }

                let task_span = tracing::info_span!("execute_task", task_id = %task.id);
                let _enter = task_span.enter();

                match sandbox.execute(&task, &lease, Arc::clone(&client)).await {
                    Ok(outcome) => {
                        let status_name = format!("{:?}", outcome.status);
                        tracing::info!(task_id = %task.id, status = %status_name, "Task executed");
                        if let Err(e) = client
                            .report_status(
                                task.id,
                                lease.id,
                                &status_name,
                                outcome.logs_ref,
                                outcome.result_ref,
                                outcome.last_error_message,
                            )
                            .await
                        {
                            tracing::error!(task_id = %task.id, status = %status_name, "Failed to report status: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!(task_id = %task.id, "Execution failed: {}", e);
                        if let Err(err) = client
                            .report_status(
                                task.id,
                                lease.id,
                                "Failed",
                                None,
                                None,
                                Some(e),
                            )
                            .await
                        {
                            tracing::error!(task_id = %task.id, "Failed to report Failed status: {}", err);
                        }
                    }
                }
            }
            Ok(None) => {
                consecutive_empty_polls += 1;
                let delay = std::cmp::min(1 << consecutive_empty_polls, 30);
                tracing::trace!(empty_polls = consecutive_empty_polls, delay_seconds = delay, "No tasks available");
                sleep(Duration::from_secs(delay as u64)).await;
            }
            Err(e) => {
                consecutive_empty_polls += 1;
                let delay = std::cmp::min(1 << consecutive_empty_polls, 30);
                tracing::error!(
                    empty_polls = consecutive_empty_polls,
                    delay_seconds = delay,
                    "Failed to poll control plane: {}",
                    e
                );
                sleep(Duration::from_secs(delay as u64)).await;
            }
        }
    }
}