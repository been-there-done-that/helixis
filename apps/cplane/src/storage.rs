use object_store::aws::AmazonS3Builder;
use object_store::path::Path as StorePath;
use object_store::{ObjectStore, PutPayload};
use std::sync::Arc;

pub fn build_s3_store(
    endpoint: &str,
    access_key: &str,
    secret_key: &str,
    bucket: &str,
) -> Arc<dyn ObjectStore> {
    let store = AmazonS3Builder::new()
        .with_endpoint(endpoint)
        .with_access_key_id(access_key)
        .with_secret_access_key(secret_key)
        .with_region("us-east-1")
        .with_bucket_name(bucket)
        .with_allow_http(true)
        .with_virtual_hosted_style_request(false)
        .build()
        .expect("failed to initialize object store");

    Arc::new(store)
}

pub async fn put_bytes(
    store: &Arc<dyn ObjectStore>,
    key: &str,
    body: Vec<u8>,
) -> Result<(), object_store::Error> {
    store
        .put(&StorePath::from(key), PutPayload::from_bytes(body.into()))
        .await?;
    Ok(())
}
