use domain::{Task, TaskLease};
use protocol::api::{PollRequest, PollResponse};
use reqwest::{Client, Url};
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
}

impl CplaneClient {
    pub fn new(base_url: &str, executor_id: Uuid) -> Result<Self, ClientError> {
        Ok(Self {
            client: Client::new(),
            base_url: Url::parse(base_url).map_err(|e| ClientError::Url(e.to_string()))?,
            executor_id,
        })
    }

    pub async fn poll_task(
        &self,
        runtime_pack_id: &str,
        lease_duration_sec: i32,
    ) -> Result<Option<(Task, TaskLease)>, ClientError> {
        let url = self
            .base_url
            .join("/v1/executors/poll")
            .map_err(|e| ClientError::Url(e.to_string()))?;

        let req = PollRequest {
            runtime_pack_id: runtime_pack_id.to_string(),
            executor_id: self.executor_id,
            lease_duration_sec,
        };

        let response = self.client.post(url).json(&req).send().await?;

        let poll_response: PollResponse = response.json().await?;

        if let (Some(task), Some(lease)) = (poll_response.task, poll_response.lease) {
            Ok(Some((task, lease)))
        } else {
            Ok(None)
        }
    }

    pub async fn report_status(&self, task_id: Uuid, status: &str) -> Result<(), ClientError> {
        let url = self
            .base_url
            .join(&format!("/v1/tasks/{}/status", task_id))
            .map_err(|e| ClientError::Url(e.to_string()))?;

        use protocol::api::TaskStatusUpdateRequest;

        let req = TaskStatusUpdateRequest {
            status: status.to_string(),
        };

        self.client.post(url).json(&req).send().await?;

        Ok(())
    }
}
