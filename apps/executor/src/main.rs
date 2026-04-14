mod artifact_client;
mod client;
mod output_store;
mod sandbox;

use artifact_client::ArtifactDownloader;
use client::CplaneClient;
use output_store::OutputUploader;
use runtime_core::RuntimeAdapter;
use runtime_node::NodeRuntimeAdapter;
use runtime_python::PythonRuntimeAdapter;
use sandbox::{ProcessSandbox, TaskSandbox};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

fn runtime_adapter_for(runtime_pack_id: &str, default_command: String) -> Box<dyn RuntimeAdapter> {
    if runtime_pack_id.starts_with("node") {
        let binary = std::env::var("NODE_EXECUTOR_COMMAND").unwrap_or_else(|_| "node".to_string());
        Box::new(NodeRuntimeAdapter::new(binary))
    } else {
        Box::new(PythonRuntimeAdapter::new(default_command))
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Use a dynamic secure UUID internally mapped to runtime pack provided via env or fallback
    let executor_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    tracing::info!("Starting Helixis Executor Agent [{executor_id}]...");

    let client = Arc::new(
        CplaneClient::new("http://127.0.0.1:3000", executor_id, session_id)
            .expect("Failed to create unified Control Plane internal client"),
    );

    let s3_endpoint =
        std::env::var("HELIXIS_S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".into());
    let s3_access_key =
        std::env::var("HELIXIS_S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".into());
    let s3_secret_key =
        std::env::var("HELIXIS_S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".into());
    let artifact_bucket =
        std::env::var("HELIXIS_ARTIFACT_BUCKET").unwrap_or_else(|_| "artifacts".into());
    let output_bucket =
        std::env::var("HELIXIS_OUTPUT_BUCKET").unwrap_or_else(|_| "task-outputs".into());

    let downloader = ArtifactDownloader::new(
        &s3_endpoint,
        &s3_access_key,
        &s3_secret_key,
        &artifact_bucket,
    );
    let output_uploader = OutputUploader::new(
        &s3_endpoint,
        &s3_access_key,
        &s3_secret_key,
        &output_bucket,
        "tasks",
    );

    let env_pack_id =
        std::env::var("RUNTIME_PACK_ID").unwrap_or_else(|_| "demo-python-pack".to_string());
    let runtime_pack_id = env_pack_id.clone();
    let command = std::env::var("EXECUTOR_COMMAND").unwrap_or_else(|_| "python3".to_string());
    let sandbox = ProcessSandbox {
        downloader,
        output_uploader,
        runtime: runtime_adapter_for(&runtime_pack_id, command),
    };

    client
        .register_executor(vec![runtime_pack_id.clone()])
        .await
        .expect("Failed to register executor");

    let heartbeat_client = Arc::clone(&client);
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(15)).await;
            if let Err(err) = heartbeat_client.send_heartbeat().await {
                tracing::warn!("Executor heartbeat failed: {}", err);
            }
        }
    });

    tracing::info!(
        "Polling for tasks matching runtime_pack: {}",
        runtime_pack_id
    );

    let mut consecutive_empty_polls = 0;

    loop {
        tracing::debug!("Polling queue...");

        match client.poll_task(&runtime_pack_id, 120).await {
            Ok(Some((task, lease, artifact, environment, payload))) => {
                consecutive_empty_polls = 0;
                tracing::info!("Acquired task {}! Executing now...", task.id);

                if let Err(err) = client
                    .report_status(task.id, lease.id, "Running", None, None, None)
                    .await
                {
                    tracing::error!("Failed to mark task {} as running: {}", task.id, err);
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }

                // Execute mock task
                match sandbox
                    .execute(
                        &task,
                        &lease,
                        &artifact,
                        Arc::clone(&client),
                        environment,
                        payload,
                    )
                    .await
                {
                    Ok(outcome) => {
                        let status_name = format!("{:?}", outcome.status);
                        tracing::info!(
                            "Execution finished with status {}. Reporting to cplane...",
                            status_name
                        );
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
                            tracing::error!("Failed to report {} status: {}", status_name, e);
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "Execution infrastructure failed: {}. Reporting failure...",
                            e
                        );
                        if let Err(err) = client
                            .report_status(task.id, lease.id, "Failed", None, None, Some(e))
                            .await
                        {
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
