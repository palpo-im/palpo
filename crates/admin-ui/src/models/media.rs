//! Media management models

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Media file information
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub mxc_uri: String,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: u64,
    pub upload_time: u64,
    pub uploader: Option<String>,
    pub is_local: bool,
    pub is_quarantined: bool,
    pub room_id: Option<String>,
    pub event_id: Option<String>,
}

/// Media statistics response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaStatsResponse {
    pub total_files: u64,
    pub total_size: u64,
    pub local_files: u64,
    pub local_size: u64,
    pub remote_files: u64,
    pub remote_size: u64,
    pub quarantined_files: u64,
    pub quarantined_size: u64,
    pub oldest_file_timestamp: Option<u64>,
    pub newest_file_timestamp: Option<u64>,
}

/// Media list request with filtering and pagination
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaListRequest {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub search: Option<String>,
    pub filter_local: Option<bool>,
    pub filter_quarantined: Option<bool>,
    pub filter_content_type: Option<String>,
    pub filter_uploader: Option<String>,
    pub filter_room_id: Option<String>,
    pub date_from: Option<u64>,
    pub date_to: Option<u64>,
    pub sort_by: Option<MediaSortField>,
    pub sort_order: Option<MediaSortOrder>,
}

/// Media list response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaListResponse {
    pub success: bool,
    pub media: Vec<MediaInfo>,
    pub total_count: u32,
    pub has_more: bool,
    pub error: Option<String>,
}

/// Media sort fields
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum MediaSortField {
    UploadTime,
    Size,
    Filename,
    ContentType,
    Uploader,
}

/// Media sort order
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum MediaSortOrder {
    Ascending,
    Descending,
}

/// Media deletion request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeleteMediaRequest {
    pub mxc_uri: String,
    pub reason: Option<String>,
}

/// Media deletion response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeleteMediaResponse {
    pub success: bool,
    pub deleted_size: Option<u64>,
    pub error: Option<String>,
}

/// Batch media deletion request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BatchDeleteMediaRequest {
    pub mxc_uris: Vec<String>,
    pub reason: Option<String>,
}

/// Batch media deletion response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BatchDeleteMediaResponse {
    pub success: bool,
    pub deleted_count: u32,
    pub failed_count: u32,
    pub total_deleted_size: u64,
    pub failed_uris: Vec<String>,
    pub errors: Vec<String>,
}

/// Media cleanup request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CleanupMediaRequest {
    pub before_timestamp: u64,
    pub include_local: bool,
    pub include_remote: bool,
    pub dry_run: bool,
    pub max_files: Option<u32>,
}

/// Media cleanup response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CleanupMediaResponse {
    pub success: bool,
    pub deleted_count: u32,
    pub total_deleted_size: u64,
    pub estimated_count: Option<u32>, // For dry run
    pub estimated_size: Option<u64>,  // For dry run
    pub error: Option<String>,
}

/// Media quarantine request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QuarantineMediaRequest {
    pub mxc_uri: String,
    pub reason: String,
}

/// Media quarantine response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QuarantineMediaResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Media unquarantine request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UnquarantineMediaRequest {
    pub mxc_uri: String,
}

/// Media unquarantine response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UnquarantineMediaResponse {
    pub success: bool,
    pub error: Option<String>,
}

/// Media storage statistics by content type
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaStorageByType {
    pub content_type: String,
    pub file_count: u64,
    pub total_size: u64,
    pub percentage: f64,
}

/// Media storage statistics by uploader
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MediaStorageByUploader {
    pub uploader: String,
    pub file_count: u64,
    pub total_size: u64,
    pub percentage: f64,
}

/// Detailed media statistics response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DetailedMediaStatsResponse {
    pub basic_stats: MediaStatsResponse,
    pub by_content_type: Vec<MediaStorageByType>,
    pub by_uploader: Vec<MediaStorageByUploader>,
    pub daily_uploads: Vec<DailyUploadStats>,
}

/// Daily upload statistics
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DailyUploadStats {
    pub date: String, // YYYY-MM-DD format
    pub file_count: u64,
    pub total_size: u64,
}

impl MediaInfo {
    /// Check if the media file is old (older than specified days)
    pub fn is_older_than_days(&self, days: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        let threshold = now - (days * 86400); // 86400 seconds in a day
        self.upload_time < threshold
    }

    /// Get human-readable file size
    pub fn human_readable_size(&self) -> String {
        let size = self.size_bytes as f64;
        
        if size < 1024.0 {
            format!("{} B", size)
        } else if size < 1024.0 * 1024.0 {
            format!("{:.1} KB", size / 1024.0)
        } else if size < 1024.0 * 1024.0 * 1024.0 {
            format!("{:.1} MB", size / (1024.0 * 1024.0))
        } else {
            format!("{:.1} GB", size / (1024.0 * 1024.0 * 1024.0))
        }
    }

    /// Get file extension from filename or content type
    pub fn get_file_extension(&self) -> Option<String> {
        if let Some(filename) = &self.filename {
            if let Some(dot_pos) = filename.rfind('.') {
                return Some(filename[dot_pos + 1..].to_lowercase());
            }
        }
        
        // Fallback to content type
        match self.content_type.as_str() {
            "image/jpeg" => Some("jpg".to_string()),
            "image/png" => Some("png".to_string()),
            "image/gif" => Some("gif".to_string()),
            "image/webp" => Some("webp".to_string()),
            "video/mp4" => Some("mp4".to_string()),
            "video/webm" => Some("webm".to_string()),
            "audio/mpeg" => Some("mp3".to_string()),
            "audio/ogg" => Some("ogg".to_string()),
            "application/pdf" => Some("pdf".to_string()),
            _ => None,
        }
    }

    /// Check if the media is an image
    pub fn is_image(&self) -> bool {
        self.content_type.starts_with("image/")
    }

    /// Check if the media is a video
    pub fn is_video(&self) -> bool {
        self.content_type.starts_with("video/")
    }

    /// Check if the media is audio
    pub fn is_audio(&self) -> bool {
        self.content_type.starts_with("audio/")
    }
}

impl Default for MediaListRequest {
    fn default() -> Self {
        Self {
            limit: Some(50),
            offset: Some(0),
            search: None,
            filter_local: None,
            filter_quarantined: Some(false), // By default, don't show quarantined files
            filter_content_type: None,
            filter_uploader: None,
            filter_room_id: None,
            date_from: None,
            date_to: None,
            sort_by: Some(MediaSortField::UploadTime),
            sort_order: Some(MediaSortOrder::Descending),
        }
    }
}

impl MediaSortField {
    /// Get human-readable description of the sort field
    pub fn description(&self) -> &'static str {
        match self {
            MediaSortField::UploadTime => "Upload Time",
            MediaSortField::Size => "File Size",
            MediaSortField::Filename => "Filename",
            MediaSortField::ContentType => "Content Type",
            MediaSortField::Uploader => "Uploader",
        }
    }
}

impl MediaStatsResponse {
    /// Calculate the percentage of local files
    pub fn local_percentage(&self) -> f64 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.local_files as f64 / self.total_files as f64) * 100.0
        }
    }

    /// Calculate the percentage of remote files
    pub fn remote_percentage(&self) -> f64 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.remote_files as f64 / self.total_files as f64) * 100.0
        }
    }

    /// Calculate the percentage of quarantined files
    pub fn quarantined_percentage(&self) -> f64 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.quarantined_files as f64 / self.total_files as f64) * 100.0
        }
    }

    /// Get human-readable total size
    pub fn human_readable_total_size(&self) -> String {
        Self::format_size(self.total_size)
    }

    /// Get human-readable local size
    pub fn human_readable_local_size(&self) -> String {
        Self::format_size(self.local_size)
    }

    /// Get human-readable remote size
    pub fn human_readable_remote_size(&self) -> String {
        Self::format_size(self.remote_size)
    }

    /// Format size in human-readable format
    fn format_size(size: u64) -> String {
        let size = size as f64;
        
        if size < 1024.0 {
            format!("{} B", size)
        } else if size < 1024.0 * 1024.0 {
            format!("{:.1} KB", size / 1024.0)
        } else if size < 1024.0 * 1024.0 * 1024.0 {
            format!("{:.1} MB", size / (1024.0 * 1024.0))
        } else if size < 1024.0 * 1024.0 * 1024.0 * 1024.0 {
            format!("{:.1} GB", size / (1024.0 * 1024.0 * 1024.0))
        } else {
            format!("{:.1} TB", size / (1024.0 * 1024.0 * 1024.0 * 1024.0))
        }
    }
}