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
    
    let executor_id = Uuid::new_v4();
    tracing::info!("Starting Helixis Executor Agent [{executor_id}]...");

    let client = CplaneClient::new("http://127.0.0.1:3000", executor_id)
        .expect("Failed to create control plane client");
        
    let downloader = ArtifactDownloader::new(
        "http://localhost:9000",
        "minioadmin",
        "minioadmin",
        "artifacts"
    );
        
    let sandbox = ProcessSandbox { downloader };
    let runtime_pack = "python-3.11-v1"; // The runtime pack this executor can handle

    tracing::info!("Polling for tasks matching runtime_pack: {}", runtime_pack);

    let mut consecutive_empty_polls = 0;

    loop {
        tracing::debug!("Polling queue...");
        
        match client.poll_task(runtime_pack, 120).await {
            Ok(Some((task, _lease))) => {
                consecutive_empty_polls = 0;
                tracing::info!("Acquired task {}! Executing now...", task.id);
                
                // Execute mock task
                match sandbox.execute(&task).await {
                    Ok(_) => tracing::info!("Execution Succeeded. (TODO: Report status to cplane)"),
                    Err(e) => tracing::error!("Execution Failed: {}", e),
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
                tracing::error!("Failed to poll control plane: {}. Retrying in {} seconds...", e, delay);
                sleep(Duration::from_secs(delay as u64)).await;
            }
        }
    }
}
