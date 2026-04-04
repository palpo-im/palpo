use serde::Deserialize;

/// Storage backend configuration for media files.
///
/// Uses an internally tagged enum so the TOML looks like:
///
/// ```toml
/// [storage]
/// backend = "fs"
/// root = "./space"
/// ```
///
/// or:
///
/// ```toml
/// [storage]
/// backend = "s3"
/// bucket = "my-bucket"
/// region = "us-east-1"
/// ```
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "backend", rename_all = "lowercase")]
pub enum StorageConfig {
    /// Local filesystem storage.
    Fs {
        /// Root directory for media files. Defaults to "./space".
        #[serde(default = "default_fs_root")]
        root: String,
    },

    /// S3-compatible object storage (AWS S3, Cloudflare R2, MinIO, etc.).
    S3 {
        /// S3 bucket name.
        bucket: String,

        /// S3 region. Defaults to "us-east-1".
        #[serde(default = "default_s3_region")]
        region: String,

        /// S3 endpoint URL. Required for non-AWS S3-compatible services.
        /// Example: "https://<account_id>.r2.cloudflarestorage.com"
        #[serde(default)]
        endpoint: Option<String>,

        /// S3 access key ID.
        #[serde(default)]
        access_key_id: Option<String>,

        /// S3 secret access key.
        #[serde(default)]
        secret_access_key: Option<String>,

        /// Object key prefix. Defaults to "media/".
        #[serde(default = "default_s3_prefix")]
        prefix: String,

        /// Enable path-style access (required for MinIO). Defaults to false.
        #[serde(default)]
        path_style: bool,
    },
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self::Fs {
            root: default_fs_root(),
        }
    }
}

fn default_fs_root() -> String {
    "./space".to_owned()
}

fn default_s3_region() -> String {
    "us-east-1".to_owned()
}

fn default_s3_prefix() -> String {
    "media/".to_owned()
}
