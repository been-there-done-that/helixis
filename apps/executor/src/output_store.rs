use bytes::Bytes;
use object_store::aws::AmazonS3Builder;
use object_store::path::Path as StorePath;
use object_store::{ObjectStore, PutPayload};
use std::sync::Arc;
use uuid::Uuid;

pub struct OutputUploader {
    store: Arc<dyn ObjectStore>,
    prefix: String,
}

impl OutputUploader {
    pub fn new(
        endpoint: &str,
        access_key: &str,
        secret_key: &str,
        bucket: &str,
        prefix: &str,
    ) -> Self {
        let store = AmazonS3Builder::new()
            .with_endpoint(endpoint)
            .with_access_key_id(access_key)
            .with_secret_access_key(secret_key)
            .with_region("us-east-1")
            .with_bucket_name(bucket)
            .with_allow_http(true)
            .with_virtual_hosted_style_request(false)
            .build()
            .expect("failed to initialize output store");

        Self {
            store: Arc::new(store),
            prefix: prefix.to_string(),
        }
    }

    pub async fn upload_outputs(
        &self,
        task_id: Uuid,
        lease_id: Uuid,
        logs: String,
        result_json: String,
    ) -> Result<(String, String), object_store::Error> {
        let logs_key = format!("{}/{}/{}/logs.txt", self.prefix, task_id, lease_id);
        let result_key = format!("{}/{}/{}/result.json", self.prefix, task_id, lease_id);

        self.store
            .put(
                &StorePath::from(logs_key.as_str()),
                PutPayload::from_bytes(Bytes::from(logs.into_bytes())),
            )
            .await?;
        self.store
            .put(
                &StorePath::from(result_key.as_str()),
                PutPayload::from_bytes(Bytes::from(result_json.into_bytes())),
            )
            .await?;

        Ok((logs_key, result_key))
    }
}
