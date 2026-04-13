mod client;
mod sandbox;

use client::CplaneClient;
use sandbox::{MockSandbox, TaskSandbox};
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
        
    let sandbox = MockSandbox;
    let runtime_pack = "python-3.11-v1"; // The runtime pack this executor can handle

    tracing::info!("Polling for tasks matching runtime_pack: {}", runtime_pack);

    loop {
        tracing::debug!("Polling queue...");
        
        match client.poll_task(runtime_pack, 120).await {
            Ok(Some((task, _lease))) => {
                tracing::info!("Acquired task {}! Executing now...", task.id);
                
                // Execute mock task
                match sandbox.execute(&task).await {
                    Ok(_) => tracing::info!("Execution Succeeded. (TODO: Report status to cplane)"),
                    Err(e) => tracing::error!("Execution Failed: {}", e),
                }
            }
            Ok(None) => {
                tracing::trace!("Queue is empty. Sleeping 3 seconds...");
                sleep(Duration::from_secs(3)).await;
            }
            Err(e) => {
                tracing::error!("Failed to poll control plane: {}. Retrying in 5 seconds...", e);
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
