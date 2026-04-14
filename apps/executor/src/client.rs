use domain::{Artifact, Task, TaskLease, TaskStatus};
use protocol::api::{
    HeartbeatRequest, LiveLogChunkRequest, PollRequest, PollResponse, RegisterExecutorRequest,
    TaskStatusUpdateRequest,
};
use reqwest::{Client, Url};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("URL parsing error: {0}")]
    Url(String),
}

pub struct CplaneClient {
    client: Client,
    base_url: Url,
    executor_id: Uuid,
    session_id: Uuid,
}

impl CplaneClient {
    pub fn new(base_url: &str, executor_id: Uuid, session_id: Uuid) -> Result<Self, ClientError> {
        Ok(Self {
            client: Client::new(),
            base_url: Url::parse(base_url).map_err(|e| ClientError::Url(e.to_string()))?,
            executor_id,
            session_id,
        })
    }

    pub async fn register_executor(&self, capabilities: Vec<String>) -> Result<(), ClientError> {
        let url = self
            .base_url
            .join("/v1/executors/register")
            .map_err(|e| ClientError::Url(e.to_string()))?;

        let req = RegisterExecutorRequest {
            executor_id: self.executor_id,
            session_id: self.session_id,
            capabilities,
        };

        self.client
            .post(url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }

    pub async fn send_heartbeat(&self) -> Result<(), ClientError> {
        let url = self
            .base_url
            .join("/v1/executors/heartbeat")
            .map_err(|e| ClientError::Url(e.to_string()))?;

        let req = HeartbeatRequest {
            executor_id: self.executor_id,
        };

        self.client
            .post(url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }

    pub async fn poll_task(
        &self,
        runtime_pack_id: &str,
        lease_duration_sec: i32,
    ) -> Result<
        Option<(
            Task,
            TaskLease,
            Artifact,
            BTreeMap<String, String>,
            Option<Value>,
        )>,
        ClientError,
    > {
        let url = self
            .base_url
            .join("/v1/executors/poll")
            .map_err(|e| ClientError::Url(e.to_string()))?;

        let req = PollRequest {
            runtime_pack_id: runtime_pack_id.to_string(),
            executor_id: self.executor_id,
            lease_duration_sec,
        };

        let response = self
            .client
            .post(url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?;

        let poll_response: PollResponse = response.json().await?;

        if let (Some(task), Some(lease), Some(artifact)) = (
            poll_response.task,
            poll_response.lease,
            poll_response.artifact,
        ) {
            Ok(Some((
                task,
                lease,
                artifact,
                poll_response.environment,
                poll_response.payload,
            )))
        } else {
            Ok(None)
        }
    }

    pub async fn report_status(
        &self,
        task_id: Uuid,
        lease_id: Uuid,
        status: &str,
        logs_ref: Option<String>,
        result_ref: Option<String>,
        last_error_message: Option<String>,
    ) -> Result<(), ClientError> {
        let url = self
            .base_url
            .join(&format!("/v1/tasks/{}/status", task_id))
            .map_err(|e| ClientError::Url(e.to_string()))?;

        let req = TaskStatusUpdateRequest {
            status: status.to_string(),
            lease_id,
            executor_id: self.executor_id,
            logs_ref,
            result_ref,
            last_error_message,
        };

        self.client
            .post(url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }

    pub async fn get_task_status(&self, task_id: Uuid) -> Result<TaskStatus, ClientError> {
        let url = self
            .base_url
            .join(&format!("/v1/tasks/{task_id}"))
            .map_err(|e| ClientError::Url(e.to_string()))?;

        let response = self.client.get(url).send().await?.error_for_status()?;
        let task_response: protocol::api::TaskResponse = response.json().await?;
        Ok(task_response.status)
    }

    pub async fn append_log_chunk(
        &self,
        task_id: Uuid,
        lease_id: Uuid,
        stream: &str,
        chunk: String,
    ) -> Result<(), ClientError> {
        let url = self
            .base_url
            .join(&format!("/v1/tasks/{task_id}/logs/append"))
            .map_err(|e| ClientError::Url(e.to_string()))?;

        let req = LiveLogChunkRequest {
            lease_id,
            executor_id: self.executor_id,
            stream: stream.to_string(),
            chunk,
        };

        self.client
            .post(url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}
