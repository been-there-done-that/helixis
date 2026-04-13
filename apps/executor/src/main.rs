mod artifact_client;
mod client;
mod sandbox;

use artifact_client::ArtifactDownloader;
use client::CplaneClient;
use sandbox::{ProcessSandbox, TaskSandbox};
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Use a dynamic secure UUID internally mapped to runtime pack provided via env or fallback
    let executor_id = Uuid::new_v4();
    tracing::info!("Starting Helixis Executor Agent [{executor_id}]...");

    let client = CplaneClient::new("http://127.0.0.1:3000", executor_id)
        .expect("Failed to create unified Control Plane internal client");

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
    let runtime_pack_id = env_pack_id.as_str();

    tracing::info!(
        "Polling for tasks matching runtime_pack: {}",
        runtime_pack_id
    );

    let mut consecutive_empty_polls = 0;

    loop {
        tracing::debug!("Polling queue...");

        match client.poll_task(runtime_pack_id, 120).await {
            Ok(Some((task, lease))) => {
                consecutive_empty_polls = 0;
                tracing::info!("Acquired task {}! Executing now...", task.id);

                // Execute mock task
                match sandbox.execute(&task).await {
                    Ok(_) => {
                        tracing::info!("Execution Succeeded. Reporting status to cplane...");
                        if let Err(e) = client.report_status(task.id, lease.id, "Succeeded").await
                        {
                            tracing::error!("Failed to report Succeeded status: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Execution Failed: {}. Reporting status to cplane...", e);
                        if let Err(err) = client.report_status(task.id, lease.id, "Failed").await {
                            tracing::error!("Failed to report Failed status: {}", err);
                        }
                    }
                }
            }
            Ok(None) => {
                consecutive_empty_polls += 1;
                // Exponential backoff: 2, 4, 8, 16... max 30 seconds
                let delay = std::cmp::min(1 << consecutive_empty_polls, 30);
                tracing::trace!("Queue is empty. Sleeping {} seconds...", delay);
                sleep(Duration::from_secs(delay as u64)).await;
            }
            Err(e) => {
                consecutive_empty_polls += 1;
                let delay = std::cmp::min(1 << consecutive_empty_polls, 30);
                tracing::error!(
                    "Failed to poll control plane: {}. Retrying in {} seconds...",
                    e,
                    delay
                );
                sleep(Duration::from_secs(delay as u64)).await;
            }
        }
    }
}
