use bytes::Bytes;
use domain::Artifact;
use object_store::aws::AmazonS3Builder;
use object_store::{ObjectStore, path::Path as StorePath};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("Storage error: {0}")]
    Store(#[from] object_store::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Archive error: {0}")]
    InvalidArchive(String),
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
            .with_allow_http(true) // Required for raw minio
            .with_virtual_hosted_style_request(false) // Required for local minio IPs
            .build()
            .expect("Failed to initialize AmazonS3 client");

        Self {
            store: Arc::new(store),
            bucket: bucket.to_string(),
        }
    }

    pub async fn ensure_artifact_cached(
        &self,
        artifact: &Artifact,
    ) -> Result<PathBuf, ArtifactError> {
        let object_key = artifact
            .object_key
            .clone()
            .ok_or_else(|| ArtifactError::InvalidArchive("artifact missing object key".into()))?;
        let cache_dir = Path::new("/tmp/helixis/cache/artifacts").join(artifact.id.to_string());

        if fs::try_exists(&cache_dir).await? {
            tracing::debug!("Artifact {} found in cache. Warm start!", artifact.id);
            return Ok(cache_dir);
        }

        tracing::info!(
            "Cache miss for {}. Fetching object: s3://{}/{}",
            artifact.id,
            self.bucket,
            object_key
        );

        let path = StorePath::from(object_key.as_str());
        let body = self.store.get(&path).await?;
        let bytes: Bytes = body.bytes().await?;

        let staging_dir = Path::new("/tmp/helixis/cache/artifacts")
            .join(format!("{object_key}.staging-{}", Uuid::new_v4()));
        fs::create_dir_all(&staging_dir).await?;

        let dir_clone = staging_dir.clone();
        tokio::task::spawn_blocking(move || {
            use flate2::read::GzDecoder;
            use std::io::Cursor;
            use tar::Archive;

            let tar = GzDecoder::new(Cursor::new(bytes.to_vec()));
            let mut archive = Archive::new(tar);

            for entry in archive.entries()? {
                let mut entry = entry?;
                let path = entry.path()?.into_owned();

                validate_archive_path(&path).map_err(std::io::Error::other)?;

                let entry_type = entry.header().entry_type();
                if entry_type.is_symlink() || entry_type.is_hard_link() {
                    return Err(std::io::Error::other(format!(
                        "unsupported archive entry type for {}",
                        path.display()
                    )));
                }

                if !entry.unpack_in(&dir_clone)? {
                    return Err(std::io::Error::other(format!(
                        "archive entry escaped extraction root: {}",
                        path.display()
                    )));
                }
            }

            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))??;

        match fs::rename(&staging_dir, &cache_dir).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_dir_all(&staging_dir).await;
            }
            Err(err) => {
                let _ = fs::remove_dir_all(&staging_dir).await;
                return Err(err.into());
            }
        }

        tracing::info!(
            "Successfully unpacked artifact {} to {:?}",
            artifact.id,
            cache_dir
        );

        Ok(cache_dir)
    }
}

fn validate_archive_path(path: &Path) -> Result<(), ArtifactError> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ArtifactError::InvalidArchive(format!(
                    "refusing unsafe archive path {}",
                    path.display()
                )));
            }
        }
    }

    Ok(())
}
