//! Media administration API implementation

use crate::models::{
    MediaInfo, MediaStatsResponse, MediaListRequest, MediaListResponse, MediaSortField, MediaSortOrder,
    DeleteMediaRequest, DeleteMediaResponse, BatchDeleteMediaRequest, BatchDeleteMediaResponse,
    CleanupMediaRequest, CleanupMediaResponse, QuarantineMediaRequest, QuarantineMediaResponse,
    UnquarantineMediaRequest, UnquarantineMediaResponse, DetailedMediaStatsResponse,
    MediaStorageByType, MediaStorageByUploader, DailyUploadStats,
    WebConfigError, AuditAction, AuditTargetType,
};
use crate::utils::audit_logger::AuditLogger;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Media administration API service
#[derive(Clone)]
pub struct MediaAdminAPI {
    audit_logger: AuditLogger,
    // In a real implementation, this would connect to the Matrix server's media storage
    // For now, we'll use in-memory storage for demonstration
    media_files: std::sync::Arc<std::sync::RwLock<HashMap<String, MediaInfo>>>,
}

impl MediaAdminAPI {
    /// Create a new MediaAdminAPI instance
    pub fn new(audit_logger: AuditLogger) -> Self {
        let media_files = std::sync::Arc::new(std::sync::RwLock::new(HashMap::new()));
        
        // Add some sample media files for demonstration
        let mut files_map = media_files.write().unwrap();
        
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        
        // Sample image file
        files_map.insert(
            "mxc://example.com/image1".to_string(),
            MediaInfo {
                mxc_uri: "mxc://example.com/image1".to_string(),
                filename: Some("profile_picture.jpg".to_string()),
                content_type: "image/jpeg".to_string(),
                size_bytes: 1024 * 512, // 512 KB
                upload_time: now - 86400, // 1 day ago
                uploader: Some("@user1:example.com".to_string()),
                is_local: true,
                is_quarantined: false,
                room_id: Some("!room1:example.com".to_string()),
                event_id: Some("$event1:example.com".to_string()),
            }
        );
        
        // Sample video file
        files_map.insert(
            "mxc://example.com/video1".to_string(),
            MediaInfo {
                mxc_uri: "mxc://example.com/video1".to_string(),
                filename: Some("meeting_recording.mp4".to_string()),
                content_type: "video/mp4".to_string(),
                size_bytes: 1024 * 1024 * 50, // 50 MB
                upload_time: now - 3600, // 1 hour ago
                uploader: Some("@admin:example.com".to_string()),
                is_local: true,
                is_quarantined: false,
                room_id: Some("!room2:example.com".to_string()),
                event_id: Some("$event2:example.com".to_string()),
            }
        );
        
        // Sample remote file
        files_map.insert(
            "mxc://remote.com/file1".to_string(),
            MediaInfo {
                mxc_uri: "mxc://remote.com/file1".to_string(),
                filename: Some("document.pdf".to_string()),
                content_type: "application/pdf".to_string(),
                size_bytes: 1024 * 1024 * 2, // 2 MB
                upload_time: now - 7200, // 2 hours ago
                uploader: Some("@user2:remote.com".to_string()),
                is_local: false,
                is_quarantined: false,
                room_id: Some("!room1:example.com".to_string()),
                event_id: Some("$event3:example.com".to_string()),
            }
        );
        
        // Sample quarantined file
        files_map.insert(
            "mxc://example.com/quarantined1".to_string(),
            MediaInfo {
                mxc_uri: "mxc://example.com/quarantined1".to_string(),
                filename: Some("suspicious_file.exe".to_string()),
                content_type: "application/octet-stream".to_string(),
                size_bytes: 1024 * 1024, // 1 MB
                upload_time: now - 86400 * 7, // 1 week ago
                uploader: Some("@suspicious:example.com".to_string()),
                is_local: true,
                is_quarantined: true,
                room_id: Some("!room3:example.com".to_string()),
                event_id: Some("$event4:example.com".to_string()),
            }
        );
        
        drop(files_map);
        
        Self {
            audit_logger,
            media_files,
        }
    }

    /// Get media statistics
    pub async fn get_media_stats(&self, admin_user: &str) -> Result<MediaStatsResponse, WebConfigError> {
        // Check permissions
        if !self.has_media_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for media management"));
        }

        let files = self.media_files.read().map_err(|_| WebConfigError::internal("Failed to read media files"))?;
        
        let mut stats = MediaStatsResponse {
            total_files: 0,
            total_size: 0,
            local_files: 0,
            local_size: 0,
            remote_files: 0,
            remote_size: 0,
            quarantined_files: 0,
            quarantined_size: 0,
            oldest_file_timestamp: None,
            newest_file_timestamp: None,
        };
        
        for media in files.values() {
            stats.total_files += 1;
            stats.total_size += media.size_bytes;
            
            if media.is_local {
                stats.local_files += 1;
                stats.local_size += media.size_bytes;
            } else {
                stats.remote_files += 1;
                stats.remote_size += media.size_bytes;
            }
            
            if media.is_quarantined {
                stats.quarantined_files += 1;
                stats.quarantined_size += media.size_bytes;
            }
            
            // Track oldest and newest files
            if stats.oldest_file_timestamp.is_none() || media.upload_time < stats.oldest_file_timestamp.unwrap() {
                stats.oldest_file_timestamp = Some(media.upload_time);
            }
            
            if stats.newest_file_timestamp.is_none() || media.upload_time > stats.newest_file_timestamp.unwrap() {
                stats.newest_file_timestamp = Some(media.upload_time);
            }
        }
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::MediaDelete, // Using existing action since MediaStats doesn't exist
            AuditTargetType::Media,
            "media_stats",
            Some(serde_json::json!(stats)),
            "Retrieved media statistics",
        ).await;
        
        Ok(stats)
    }

    /// Get detailed media statistics
    pub async fn get_detailed_media_stats(&self, admin_user: &str) -> Result<DetailedMediaStatsResponse, WebConfigError> {
        // Check permissions
        if !self.has_media_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for media management"));
        }

        let basic_stats = self.get_media_stats(admin_user).await?;
        let files = self.media_files.read().map_err(|_| WebConfigError::internal("Failed to read media files"))?;
        
        // Calculate statistics by content type
        let mut by_content_type: HashMap<String, (u64, u64)> = HashMap::new();
        let mut by_uploader: HashMap<String, (u64, u64)> = HashMap::new();
        let mut daily_uploads: HashMap<String, (u64, u64)> = HashMap::new();
        
        for media in files.values() {
            // By content type
            let (count, size) = by_content_type.entry(media.content_type.clone()).or_insert((0, 0));
            *count += 1;
            *size += media.size_bytes;
            
            // By uploader
            if let Some(uploader) = &media.uploader {
                let (count, size) = by_uploader.entry(uploader.clone()).or_insert((0, 0));
                *count += 1;
                *size += media.size_bytes;
            }
            
            // Daily uploads
            let date = format_timestamp_to_date(media.upload_time);
            let (count, size) = daily_uploads.entry(date).or_insert((0, 0));
            *count += 1;
            *size += media.size_bytes;
        }
        
        // Convert to response format
        let mut by_content_type_vec: Vec<MediaStorageByType> = by_content_type
            .into_iter()
            .map(|(content_type, (count, size))| MediaStorageByType {
                content_type,
                file_count: count,
                total_size: size,
                percentage: if basic_stats.total_size > 0 {
                    (size as f64 / basic_stats.total_size as f64) * 100.0
                } else {
                    0.0
                },
            })
            .collect();
        by_content_type_vec.sort_by(|a, b| b.total_size.cmp(&a.total_size));
        
        let mut by_uploader_vec: Vec<MediaStorageByUploader> = by_uploader
            .into_iter()
            .map(|(uploader, (count, size))| MediaStorageByUploader {
                uploader,
                file_count: count,
                total_size: size,
                percentage: if basic_stats.total_size > 0 {
                    (size as f64 / basic_stats.total_size as f64) * 100.0
                } else {
                    0.0
                },
            })
            .collect();
        by_uploader_vec.sort_by(|a, b| b.total_size.cmp(&a.total_size));
        
        let mut daily_uploads_vec: Vec<DailyUploadStats> = daily_uploads
            .into_iter()
            .map(|(date, (count, size))| DailyUploadStats {
                date,
                file_count: count,
                total_size: size,
            })
            .collect();
        daily_uploads_vec.sort_by(|a, b| a.date.cmp(&b.date));
        
        Ok(DetailedMediaStatsResponse {
            basic_stats,
            by_content_type: by_content_type_vec,
            by_uploader: by_uploader_vec,
            daily_uploads: daily_uploads_vec,
        })
    }

    /// List media files with filtering and pagination
    pub async fn list_media(&self, request: MediaListRequest, admin_user: &str) -> Result<MediaListResponse, WebConfigError> {
        // Check permissions
        if !self.has_media_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for media management"));
        }

        let files = self.media_files.read().map_err(|_| WebConfigError::internal("Failed to read media files"))?;
        
        let mut filtered_media: Vec<MediaInfo> = files.values().cloned().collect();
        
        // Apply filters
        if let Some(search) = &request.search {
            let search_lower = search.to_lowercase();
            filtered_media.retain(|media| {
                media.filename.as_ref().map_or(false, |name| name.to_lowercase().contains(&search_lower)) ||
                media.content_type.to_lowercase().contains(&search_lower) ||
                media.mxc_uri.to_lowercase().contains(&search_lower) ||
                media.uploader.as_ref().map_or(false, |uploader| uploader.to_lowercase().contains(&search_lower))
            });
        }
        
        if let Some(filter_local) = request.filter_local {
            filtered_media.retain(|media| media.is_local == filter_local);
        }
        
        if let Some(filter_quarantined) = request.filter_quarantined {
            filtered_media.retain(|media| media.is_quarantined == filter_quarantined);
        }
        
        if let Some(filter_content_type) = &request.filter_content_type {
            filtered_media.retain(|media| media.content_type == *filter_content_type);
        }
        
        if let Some(filter_uploader) = &request.filter_uploader {
            filtered_media.retain(|media| media.uploader.as_ref() == Some(filter_uploader));
        }
        
        if let Some(filter_room_id) = &request.filter_room_id {
            filtered_media.retain(|media| media.room_id.as_ref() == Some(filter_room_id));
        }
        
        if let Some(date_from) = request.date_from {
            filtered_media.retain(|media| media.upload_time >= date_from);
        }
        
        if let Some(date_to) = request.date_to {
            filtered_media.retain(|media| media.upload_time <= date_to);
        }
        
        // Apply sorting
        if let Some(sort_by) = &request.sort_by {
            let ascending = matches!(request.sort_order, Some(MediaSortOrder::Ascending) | None);
            
            filtered_media.sort_by(|a, b| {
                let cmp = match sort_by {
                    MediaSortField::UploadTime => a.upload_time.cmp(&b.upload_time),
                    MediaSortField::Size => a.size_bytes.cmp(&b.size_bytes),
                    MediaSortField::Filename => {
                        let a_name = a.filename.as_deref().unwrap_or(&a.mxc_uri);
                        let b_name = b.filename.as_deref().unwrap_or(&b.mxc_uri);
                        a_name.cmp(b_name)
                    },
                    MediaSortField::ContentType => a.content_type.cmp(&b.content_type),
                    MediaSortField::Uploader => {
                        let a_uploader = a.uploader.as_deref().unwrap_or("");
                        let b_uploader = b.uploader.as_deref().unwrap_or("");
                        a_uploader.cmp(b_uploader)
                    },
                };
                
                if ascending { cmp } else { cmp.reverse() }
            });
        }
        
        let total_count = filtered_media.len() as u32;
        
        // Apply pagination
        let offset = request.offset.unwrap_or(0) as usize;
        let limit = request.limit.unwrap_or(50) as usize;
        
        let paginated_media: Vec<MediaInfo> = filtered_media
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect();
        
        let has_more = (offset + paginated_media.len()) < total_count as usize;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::MediaDelete, // Using existing action since MediaList doesn't exist
            AuditTargetType::Media,
            "media_list",
            Some(serde_json::json!({
                "filter": {
                    "search": request.search,
                    "local": request.filter_local,
                    "quarantined": request.filter_quarantined,
                    "content_type": request.filter_content_type,
                    "uploader": request.filter_uploader,
                    "room_id": request.filter_room_id
                },
                "pagination": {
                    "offset": request.offset,
                    "limit": request.limit
                },
                "result_count": paginated_media.len()
            })),
            "Listed media files with filters",
        ).await;
        
        Ok(MediaListResponse {
            success: true,
            media: paginated_media,
            total_count,
            has_more,
            error: None,
        })
    }

    /// Get media file information
    pub async fn get_media_info(&self, mxc_uri: &str, admin_user: &str) -> Result<MediaInfo, WebConfigError> {
        // Check permissions
        if !self.has_media_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for media management"));
        }

        let files = self.media_files.read().map_err(|_| WebConfigError::internal("Failed to read media files"))?;
        
        let media = files.get(mxc_uri).ok_or_else(|| {
            WebConfigError::validation(format!("Media file {} not found", mxc_uri))
        })?;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::MediaDelete, // Using existing action since MediaInfo doesn't exist
            AuditTargetType::Media,
            mxc_uri,
            Some(serde_json::json!({
                "size_bytes": media.size_bytes,
                "content_type": media.content_type,
                "is_local": media.is_local,
                "is_quarantined": media.is_quarantined
            })),
            &format!("Retrieved media info for {}", mxc_uri),
        ).await;
        
        Ok(media.clone())
    }

    /// Delete a single media file
    pub async fn delete_media(&self, request: DeleteMediaRequest, admin_user: &str) -> Result<DeleteMediaResponse, WebConfigError> {
        // Check permissions
        if !self.has_media_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for media management"));
        }

        let mut files = self.media_files.write().map_err(|_| WebConfigError::internal("Failed to write media files"))?;
        
        let media = files.remove(&request.mxc_uri).ok_or_else(|| {
            WebConfigError::validation(format!("Media file {} not found", request.mxc_uri))
        })?;
        
        let deleted_size = media.size_bytes;
        drop(files);
        
        // In a real implementation, this would delete the actual file from storage
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::MediaDelete,
            AuditTargetType::Media,
            &request.mxc_uri,
            Some(serde_json::json!({
                "deleted_size": deleted_size,
                "reason": request.reason,
                "filename": media.filename,
                "content_type": media.content_type,
                "uploader": media.uploader
            })),
            &format!("Deleted media file {}", request.mxc_uri),
        ).await;
        
        Ok(DeleteMediaResponse {
            success: true,
            deleted_size: Some(deleted_size),
            error: None,
        })
    }

    /// Delete multiple media files
    pub async fn delete_media_batch(&self, request: BatchDeleteMediaRequest, admin_user: &str) -> Result<BatchDeleteMediaResponse, WebConfigError> {
        // Check permissions
        if !self.has_media_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for media management"));
        }

        let mut deleted_count = 0;
        let mut failed_count = 0;
        let mut total_deleted_size = 0;
        let mut failed_uris = Vec::new();
        let mut errors = Vec::new();
        
        for mxc_uri in &request.mxc_uris {
            let delete_request = DeleteMediaRequest {
                mxc_uri: mxc_uri.clone(),
                reason: request.reason.clone(),
            };
            
            match self.delete_media(delete_request, admin_user).await {
                Ok(response) => {
                    deleted_count += 1;
                    if let Some(size) = response.deleted_size {
                        total_deleted_size += size;
                    }
                },
                Err(e) => {
                    failed_count += 1;
                    failed_uris.push(mxc_uri.clone());
                    errors.push(format!("{}: {}", mxc_uri, e));
                }
            }
        }
        
        // Log the batch operation
        self.audit_logger.log_action(
            admin_user,
            AuditAction::MediaDelete,
            AuditTargetType::Media,
            "batch_delete",
            Some(serde_json::json!({
                "total_count": request.mxc_uris.len(),
                "deleted_count": deleted_count,
                "failed_count": failed_count,
                "total_deleted_size": total_deleted_size,
                "reason": request.reason,
                "failed_uris": failed_uris
            })),
            &format!("Batch deleted {} media files", request.mxc_uris.len()),
        ).await;
        
        Ok(BatchDeleteMediaResponse {
            success: failed_count == 0,
            deleted_count,
            failed_count,
            total_deleted_size,
            failed_uris,
            errors,
        })
    }

    /// Clean up old media files
    pub async fn cleanup_old_media(&self, request: CleanupMediaRequest, admin_user: &str) -> Result<CleanupMediaResponse, WebConfigError> {
        // Check permissions
        if !self.has_media_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for media management"));
        }

        if request.dry_run {
            // For dry run, we only need read access
            let files = self.media_files.read().map_err(|_| WebConfigError::internal("Failed to read media files"))?;
            
            // Find files to delete
            let mut files_to_delete = Vec::new();
            for (mxc_uri, media) in files.iter() {
                if media.upload_time < request.before_timestamp {
                    if (request.include_local && media.is_local) || (request.include_remote && !media.is_local) {
                        files_to_delete.push((mxc_uri.clone(), media.clone()));
                    }
                }
            }
            
            // Apply max_files limit
            if let Some(max_files) = request.max_files {
                files_to_delete.truncate(max_files as usize);
            }
            
            let estimated_count = files_to_delete.len() as u32;
            let estimated_size: u64 = files_to_delete.iter().map(|(_, media)| media.size_bytes).sum();
            
            drop(files);
            
            // Log the dry run
            self.audit_logger.log_action(
                admin_user,
                AuditAction::MediaDelete,
                AuditTargetType::Media,
                "cleanup_dry_run",
                Some(serde_json::json!({
                    "before_timestamp": request.before_timestamp,
                    "include_local": request.include_local,
                    "include_remote": request.include_remote,
                    "estimated_count": estimated_count,
                    "estimated_size": estimated_size
                })),
                &format!("Media cleanup dry run: {} files, {} bytes", estimated_count, estimated_size),
            ).await;
            
            return Ok(CleanupMediaResponse {
                success: true,
                deleted_count: 0,
                total_deleted_size: 0,
                estimated_count: Some(estimated_count),
                estimated_size: Some(estimated_size),
                error: None,
            });
        }
        
        // For actual deletion, we need write access
        let mut files = self.media_files.write().map_err(|_| WebConfigError::internal("Failed to write media files"))?;
        
        // Find files to delete
        let mut files_to_delete = Vec::new();
        for (mxc_uri, media) in files.iter() {
            if media.upload_time < request.before_timestamp {
                if (request.include_local && media.is_local) || (request.include_remote && !media.is_local) {
                    files_to_delete.push((mxc_uri.clone(), media.clone()));
                }
            }
        }
        
        // Apply max_files limit
        if let Some(max_files) = request.max_files {
            files_to_delete.truncate(max_files as usize);
        }
        
        // Actually delete the files
        let mut deleted_count = 0;
        let mut total_deleted_size = 0;
        
        for (mxc_uri, media) in files_to_delete {
            if files.remove(&mxc_uri).is_some() {
                deleted_count += 1;
                total_deleted_size += media.size_bytes;
            }
        }
        
        drop(files);
        
        // Log the cleanup operation
        self.audit_logger.log_action(
            admin_user,
            AuditAction::MediaDelete,
            AuditTargetType::Media,
            "cleanup",
            Some(serde_json::json!({
                "before_timestamp": request.before_timestamp,
                "include_local": request.include_local,
                "include_remote": request.include_remote,
                "deleted_count": deleted_count,
                "total_deleted_size": total_deleted_size
            })),
            &format!("Cleaned up {} old media files, {} bytes", deleted_count, total_deleted_size),
        ).await;
        
        Ok(CleanupMediaResponse {
            success: true,
            deleted_count,
            total_deleted_size,
            estimated_count: None,
            estimated_size: None,
            error: None,
        })
    }

    /// Quarantine a media file
    pub async fn quarantine_media(&self, request: QuarantineMediaRequest, admin_user: &str) -> Result<QuarantineMediaResponse, WebConfigError> {
        // Check permissions
        if !self.has_media_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for media management"));
        }

        let mut files = self.media_files.write().map_err(|_| WebConfigError::internal("Failed to write media files"))?;
        
        let media = files.get_mut(&request.mxc_uri).ok_or_else(|| {
            WebConfigError::validation(format!("Media file {} not found", request.mxc_uri))
        })?;
        
        if media.is_quarantined {
            return Ok(QuarantineMediaResponse {
                success: false,
                error: Some("Media file is already quarantined".to_string()),
            });
        }
        
        media.is_quarantined = true;
        drop(files);
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::MediaDelete, // Using existing action since MediaQuarantine doesn't exist
            AuditTargetType::Media,
            &request.mxc_uri,
            Some(serde_json::json!({
                "reason": request.reason,
                "action": "quarantine"
            })),
            &format!("Quarantined media file {}", request.mxc_uri),
        ).await;
        
        Ok(QuarantineMediaResponse {
            success: true,
            error: None,
        })
    }

    /// Unquarantine a media file
    pub async fn unquarantine_media(&self, request: UnquarantineMediaRequest, admin_user: &str) -> Result<UnquarantineMediaResponse, WebConfigError> {
        // Check permissions
        if !self.has_media_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for media management"));
        }

        let mut files = self.media_files.write().map_err(|_| WebConfigError::internal("Failed to write media files"))?;
        
        let media = files.get_mut(&request.mxc_uri).ok_or_else(|| {
            WebConfigError::validation(format!("Media file {} not found", request.mxc_uri))
        })?;
        
        if !media.is_quarantined {
            return Ok(UnquarantineMediaResponse {
                success: false,
                error: Some("Media file is not quarantined".to_string()),
            });
        }
        
        media.is_quarantined = false;
        drop(files);
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::MediaDelete, // Using existing action since MediaUnquarantine doesn't exist
            AuditTargetType::Media,
            &request.mxc_uri,
            Some(serde_json::json!({
                "action": "unquarantine"
            })),
            &format!("Unquarantined media file {}", request.mxc_uri),
        ).await;
        
        Ok(UnquarantineMediaResponse {
            success: true,
            error: None,
        })
    }

    /// Check if the admin user has media management permissions
    async fn has_media_management_permission(&self, _admin_user: &str) -> Result<bool, WebConfigError> {
        // In a real implementation, this would check the admin user's permissions
        // For now, we'll assume all admin users have media management permissions
        Ok(true)
    }
}

/// Helper function to format timestamp to date string
fn format_timestamp_to_date(timestamp: u64) -> String {
    use std::time::Duration;
    
    let _datetime = UNIX_EPOCH + Duration::from_secs(timestamp);
    // In a real implementation, you would use a proper date formatting library
    // For now, we'll use a simple approximation
    let days_since_epoch = timestamp / 86400;
    let year = 1970 + (days_since_epoch / 365);
    let day_of_year = days_since_epoch % 365;
    let month = (day_of_year / 30) + 1;
    let day = (day_of_year % 30) + 1;
    
    format!("{:04}-{:02}-{:02}", year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::audit_logger::AuditLogger;

    fn create_test_api() -> MediaAdminAPI {
        let audit_logger = AuditLogger::new(1000);
        MediaAdminAPI::new(audit_logger)
    }

    #[tokio::test]
    async fn test_get_media_stats() {
        let api = create_test_api();
        
        let stats = api.get_media_stats("admin").await.unwrap();
        
        assert_eq!(stats.total_files, 4); // 4 sample files
        assert!(stats.total_size > 0);
        assert_eq!(stats.local_files, 3); // 3 local files
        assert_eq!(stats.remote_files, 1); // 1 remote file
        assert_eq!(stats.quarantined_files, 1); // 1 quarantined file
    }

    #[tokio::test]
    async fn test_list_media() {
        let api = create_test_api();
        let request = MediaListRequest::default();
        
        let response = api.list_media(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert_eq!(response.media.len(), 3); // 3 non-quarantined files by default
        assert_eq!(response.total_count, 3);
    }

    #[tokio::test]
    async fn test_list_media_with_filters() {
        let api = create_test_api();
        let request = MediaListRequest {
            filter_local: Some(true),
            filter_quarantined: Some(true), // Include quarantined files
            ..Default::default()
        };
        
        let response = api.list_media(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert_eq!(response.media.len(), 1); // 1 local quarantined file
    }

    #[tokio::test]
    async fn test_get_media_info() {
        let api = create_test_api();
        
        let media = api.get_media_info("mxc://example.com/image1", "admin").await.unwrap();
        
        assert_eq!(media.mxc_uri, "mxc://example.com/image1");
        assert_eq!(media.content_type, "image/jpeg");
        assert!(media.is_local);
        assert!(!media.is_quarantined);
    }

    #[tokio::test]
    async fn test_delete_media() {
        let api = create_test_api();
        let request = DeleteMediaRequest {
            mxc_uri: "mxc://example.com/image1".to_string(),
            reason: Some("Test deletion".to_string()),
        };
        
        let response = api.delete_media(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.deleted_size.is_some());
        
        // Verify file is deleted
        let result = api.get_media_info("mxc://example.com/image1", "admin").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_batch_delete_media() {
        let api = create_test_api();
        let request = BatchDeleteMediaRequest {
            mxc_uris: vec![
                "mxc://example.com/image1".to_string(),
                "mxc://example.com/video1".to_string(),
            ],
            reason: Some("Batch test deletion".to_string()),
        };
        
        let response = api.delete_media_batch(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert_eq!(response.deleted_count, 2);
        assert_eq!(response.failed_count, 0);
        assert!(response.total_deleted_size > 0);
    }

    #[tokio::test]
    async fn test_cleanup_old_media_dry_run() {
        let api = create_test_api();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let request = CleanupMediaRequest {
            before_timestamp: now + 3600, // 1 hour in the future (should match all files)
            include_local: true,
            include_remote: true,
            dry_run: true,
            max_files: None,
        };
        
        let response = api.cleanup_old_media(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert_eq!(response.deleted_count, 0); // Dry run doesn't delete
        assert!(response.estimated_count.is_some());
        assert!(response.estimated_size.is_some());
    }

    #[tokio::test]
    async fn test_quarantine_media() {
        let api = create_test_api();
        let request = QuarantineMediaRequest {
            mxc_uri: "mxc://example.com/image1".to_string(),
            reason: "Test quarantine".to_string(),
        };
        
        let response = api.quarantine_media(request, "admin").await.unwrap();
        
        assert!(response.success);
        
        // Verify file is quarantined
        let media = api.get_media_info("mxc://example.com/image1", "admin").await.unwrap();
        assert!(media.is_quarantined);
    }

    #[tokio::test]
    async fn test_unquarantine_media() {
        let api = create_test_api();
        
        // First quarantine a file
        let quarantine_request = QuarantineMediaRequest {
            mxc_uri: "mxc://example.com/image1".to_string(),
            reason: "Test quarantine".to_string(),
        };
        api.quarantine_media(quarantine_request, "admin").await.unwrap();
        
        // Then unquarantine it
        let unquarantine_request = UnquarantineMediaRequest {
            mxc_uri: "mxc://example.com/image1".to_string(),
        };
        let response = api.unquarantine_media(unquarantine_request, "admin").await.unwrap();
        
        assert!(response.success);
        
        // Verify file is not quarantined
        let media = api.get_media_info("mxc://example.com/image1", "admin").await.unwrap();
        assert!(!media.is_quarantined);
    }

    #[tokio::test]
    async fn test_detailed_media_stats() {
        let api = create_test_api();
        
        let stats = api.get_detailed_media_stats("admin").await.unwrap();
        
        assert!(stats.basic_stats.total_files > 0);
        assert!(!stats.by_content_type.is_empty());
        assert!(!stats.by_uploader.is_empty());
        assert!(!stats.daily_uploads.is_empty());
    }
}