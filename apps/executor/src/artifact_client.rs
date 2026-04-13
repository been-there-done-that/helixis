use bytes::Bytes;
use object_store::aws::AmazonS3Builder;
use object_store::{path::Path as StorePath, ObjectStore};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("Storage error: {0}")]
    Store(#[from] object_store::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct ArtifactDownloader {
    store: Arc<dyn ObjectStore>,
    bucket: String,
}

impl ArtifactDownloader {
    pub fn new(endpoint: &str, access_key: &str, secret_key: &str, bucket: &str) -> Self {
        tracing::info!("Initializing S3 client connected to {}", endpoint);
        
        let store = AmazonS3Builder::new()
            .with_endpoint(endpoint)
            .with_access_key_id(access_key)
            .with_secret_access_key(secret_key)
            .with_region("us-east-1") // MinIO standard default
            .with_bucket_name(bucket)
            .with_allow_http(true)    // Required for raw minio
            .build()
            .expect("Failed to initialize AmazonS3 client");
            
        Self {
            store: Arc::new(store),
            bucket: bucket.to_string(),
        }
    }

    pub async fn download_artifact(&self, artifact_id: Uuid) -> Result<PathBuf, ArtifactError> {
        // We assume object key is the UUID of the artifact
        let object_key = format!("{}", artifact_id);
        let path = StorePath::from(object_key.as_str());
        
        tracing::debug!("Fetching object: s3://{}/{}", self.bucket, object_key);
        let body = self.store.get(&path).await?;
        let bytes: Bytes = body.bytes().await?;
        
        // Write the raw file locally to /tmp/helixis-sandbox/...
        let sandbox_dir = Path::new("/tmp/helixis-sandbox").join(object_key.as_str());
        fs::create_dir_all(&sandbox_dir).await?;
        
        let artifact_path = sandbox_dir.join("payload.tar.gz"); // using .tar.gz based on context
        fs::write(&artifact_path, &bytes).await?;
        
        tracing::info!("Successfully dumped artifact to {:?}", artifact_path);
        
        Ok(artifact_path)
    }
}
