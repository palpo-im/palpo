use std::sync::OnceLock;

use opendal::{Operator, layers::LoggingLayer};

use crate::AppResult;
use crate::config::StorageConfig;

static OPERATOR: OnceLock<Operator> = OnceLock::new();

/// Initialize the global storage operator from configuration.
/// Must be called once at startup after config is loaded.
pub fn init(config: &StorageConfig) -> AppResult<()> {
    let op = build_operator(config)?;
    OPERATOR
        .set(op)
        .map_err(|_| crate::AppError::public("Storage operator already initialized"))?;
    Ok(())
}

/// Get the global storage operator.
pub fn operator() -> &'static Operator {
    OPERATOR
        .get()
        .expect("Storage operator not initialized. Call storage::init() first.")
}

fn build_operator(config: &StorageConfig) -> AppResult<Operator> {
    match config {
        StorageConfig::Fs { root } => build_fs_operator(root),
        StorageConfig::S3 {
            bucket,
            region,
            endpoint,
            access_key_id,
            secret_access_key,
            prefix,
            path_style,
        } => build_s3_operator(
            bucket,
            region,
            endpoint.as_deref(),
            access_key_id.as_deref(),
            secret_access_key.as_deref(),
            prefix,
            *path_style,
        ),
    }
}

fn build_fs_operator(root: &str) -> AppResult<Operator> {
    let builder = opendal::services::Fs::default().root(root);
    let op = Operator::new(builder)?
        .layer(LoggingLayer::default())
        .finish();
    info!("Storage backend initialized: fs (root={})", root);
    Ok(op)
}

fn build_s3_operator(
    bucket: &str,
    region: &str,
    endpoint: Option<&str>,
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
    prefix: &str,
    path_style: bool,
) -> AppResult<Operator> {
    let mut builder = opendal::services::S3::default()
        .bucket(bucket)
        .region(region);

    if let Some(endpoint) = endpoint {
        builder = builder.endpoint(endpoint);
    }
    if let Some(access_key_id) = access_key_id {
        builder = builder.access_key_id(access_key_id);
    }
    if let Some(secret_access_key) = secret_access_key {
        builder = builder.secret_access_key(secret_access_key);
    }
    if path_style {
        builder = builder.enable_virtual_host_style();
    }

    if !prefix.is_empty() {
        builder = builder.root(prefix);
    }

    let op = Operator::new(builder)?
        .layer(LoggingLayer::default())
        .finish();
    info!(
        "Storage backend initialized: s3 (bucket={}, region={}, endpoint={:?})",
        bucket, region, endpoint
    );
    Ok(op)
}

/// Build the storage key for a media file.
pub fn media_key(server_name: &str, media_id: &str) -> String {
    format!("media/{server_name}/{media_id}")
}

/// Build the storage key for a thumbnail file.
pub fn thumbnail_key(server_name: &str, media_id: &str, thumbnail_id: i64) -> String {
    format!("media/{server_name}/{media_id}.thumbnails/{thumbnail_id}")
}

/// Write bytes to storage.
pub async fn write(key: &str, data: &[u8]) -> AppResult<()> {
    operator().write(key, data.to_vec()).await?;
    Ok(())
}

/// Read bytes from storage.
pub async fn read(key: &str) -> AppResult<Vec<u8>> {
    let data = operator().read(key).await?;
    Ok(data.to_vec())
}

/// Check if an object exists in storage.
pub async fn exists(key: &str) -> AppResult<bool> {
    Ok(operator().exists(key).await?)
}

/// Delete an object from storage.
pub async fn delete(key: &str) -> AppResult<()> {
    operator().delete(key).await?;
    Ok(())
}
