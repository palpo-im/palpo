//! Admin Media API
//!
//! - GET /_synapse/admin/v1/media/{server_name}/{media_id}
//! - DELETE /_synapse/admin/v1/media/{server_name}/{media_id}
//! - GET /_synapse/admin/v1/room/{room_id}/media
//! - GET /_synapse/admin/v1/users/{user_id}/media
//! - DELETE /_synapse/admin/v1/users/{user_id}/media
//! - POST /_synapse/admin/v1/purge_media_cache
//! - POST /_synapse/admin/v1/media/delete

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::identifiers::*;
use crate::{JsonResult, MatrixError, config, data, json_ok};

pub fn router() -> Router {
    Router::new()
        .push(
            Router::with_path("v1/media/{server_name}/{media_id}")
                .get(get_media_info)
                .delete(delete_media),
        )
        .push(Router::with_path("v1/media/delete").post(delete_media_by_date_size))
        .push(Router::with_path("v1/room/{room_id}/media").get(list_media_in_room))
        .push(
            Router::with_path("v1/users/{user_id}/media")
                .get(list_user_media)
                .delete(delete_user_media),
        )
        .push(Router::with_path("v1/purge_media_cache").post(purge_media_cache))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MediaInfoResponse {
    pub media_info: MediaInfo,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MediaInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub media_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_length: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_ts: Option<i64>,
}

impl From<data::media::DbMetadata> for MediaInfo {
    fn from(m: data::media::DbMetadata) -> Self {
        MediaInfo {
            media_origin: Some(m.origin_server.to_string()),
            user_id: m.created_by.map(|u| u.to_string()),
            media_id: m.media_id,
            media_type: m.content_type,
            media_length: Some(m.file_size),
            upload_name: m.file_name,
            created_ts: Some(m.created_at.get() as i64),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoomMediaResponse {
    pub local: Vec<String>,
    pub remote: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserMediaResponse {
    pub media: Vec<MediaInfo>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteMediaResponse {
    pub deleted_media: Vec<String>,
    pub total: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PurgeMediaCacheResponse {
    pub deleted: i64,
}

#[derive(Debug, Deserialize, ToParameters)]
pub struct ListUserMediaQuery {
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub order_by: Option<String>,
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Deserialize, ToParameters)]
pub struct DeleteMediaByDateSizeQuery {
    pub before_ts: i64,
    #[serde(default)]
    pub size_gt: Option<i64>,
}

/// GET /_synapse/admin/v1/media/{server_name}/{media_id}
#[endpoint(operation_id = "get_media_info")]
pub fn get_media_info(
    server_name: PathParam<OwnedServerName>,
    media_id: PathParam<String>,
) -> JsonResult<MediaInfoResponse> {
    let server_name = server_name.into_inner();
    let media_id = media_id.into_inner();

    let metadata = data::media::get_metadata(&server_name, &media_id)?
        .ok_or_else(|| MatrixError::not_found("Unknown media"))?;

    json_ok(MediaInfoResponse {
        media_info: metadata.into(),
    })
}

/// DELETE /_synapse/admin/v1/media/{server_name}/{media_id}
#[endpoint(operation_id = "delete_media")]
pub fn delete_media(
    server_name: PathParam<OwnedServerName>,
    media_id: PathParam<String>,
) -> JsonResult<DeleteMediaResponse> {
    let server_name = server_name.into_inner();
    let media_id = media_id.into_inner();

    if server_name != *config::get().server_name {
        return Err(MatrixError::invalid_param("Can only delete local media").into());
    }

    let _ = data::media::get_metadata(&server_name, &media_id)?
        .ok_or_else(|| MatrixError::not_found("Unknown media"))?;

    let (deleted_media, total) = data::media::delete_media_by_ids(&server_name, &[media_id])?;

    json_ok(DeleteMediaResponse {
        deleted_media,
        total,
    })
}

/// POST /_synapse/admin/v1/media/delete
#[endpoint(operation_id = "delete_media_by_date_size")]
pub fn delete_media_by_date_size(
    query: DeleteMediaByDateSizeQuery,
) -> JsonResult<DeleteMediaResponse> {
    let before_ts = query.before_ts;
    let size_gt = query.size_gt.unwrap_or(0);

    if before_ts < 30000000000 {
        return Err(MatrixError::invalid_param(
            "Query parameter before_ts you provided is from the year 1970. \
             Double check that you are providing a timestamp in milliseconds.",
        )
        .into());
    }

    let local_server = &config::get().server_name;
    let (deleted_media, total) = data::media::delete_old_local_media(local_server, before_ts, size_gt)?;

    json_ok(DeleteMediaResponse {
        deleted_media,
        total,
    })
}

/// GET /_synapse/admin/v1/room/{room_id}/media
///
/// Note: This requires scanning events in the room which is not currently implemented.
/// Returns empty lists for now.
#[endpoint(operation_id = "list_media_in_room")]
pub fn list_media_in_room(room_id: PathParam<OwnedRoomId>) -> JsonResult<RoomMediaResponse> {
    let _room_id = room_id.into_inner();

    json_ok(RoomMediaResponse {
        local: vec![],
        remote: vec![],
    })
}

/// GET /_synapse/admin/v1/users/{user_id}/media
#[endpoint(operation_id = "list_user_media")]
pub fn list_user_media(
    user_id: PathParam<OwnedUserId>,
    query: ListUserMediaQuery,
) -> JsonResult<UserMediaResponse> {
    let user_id = user_id.into_inner();
    let from = query.from.unwrap_or(0);
    let limit = query.limit.unwrap_or(100).min(1000);
    let order_by = query.order_by.as_deref();
    let dir = query.dir.as_deref();

    if *user_id.server_name() != *config::get().server_name {
        return Err(MatrixError::invalid_param("Can only look up local users").into());
    }

    if !data::user::user_exists(&user_id)? {
        return Err(MatrixError::not_found("Unknown user").into());
    }

    let (media_list, total) = data::media::list_media_by_user(&user_id, from, limit, order_by, dir)?;

    let media: Vec<MediaInfo> = media_list.into_iter().map(Into::into).collect();
    let next_token = if (from + limit) < total {
        Some(from + media.len() as i64)
    } else {
        None
    };

    json_ok(UserMediaResponse {
        media,
        total,
        next_token,
    })
}

/// DELETE /_synapse/admin/v1/users/{user_id}/media
#[endpoint(operation_id = "delete_user_media")]
pub fn delete_user_media(
    user_id: PathParam<OwnedUserId>,
    query: ListUserMediaQuery,
) -> JsonResult<DeleteMediaResponse> {
    let user_id = user_id.into_inner();
    let from = query.from.unwrap_or(0);
    let limit = query.limit.unwrap_or(100).min(1000);
    let order_by = query.order_by.as_deref();
    let dir = query.dir.as_deref();

    if *user_id.server_name() != *config::get().server_name {
        return Err(MatrixError::invalid_param("Can only look up local users").into());
    }

    if !data::user::user_exists(&user_id)? {
        return Err(MatrixError::not_found("Unknown user").into());
    }

    let (media_list, _) = data::media::list_media_by_user(&user_id, from, limit, order_by, dir)?;
    let media_ids: Vec<String> = media_list.iter().map(|m| m.media_id.clone()).collect();

    let local_server = &config::get().server_name;
    let (deleted_media, total) = data::media::delete_media_by_ids(local_server, &media_ids)?;

    json_ok(DeleteMediaResponse {
        deleted_media,
        total,
    })
}

/// POST /_synapse/admin/v1/purge_media_cache
#[endpoint(operation_id = "purge_media_cache")]
pub fn purge_media_cache(before_ts: QueryParam<i64, true>) -> JsonResult<PurgeMediaCacheResponse> {
    let before_ts = before_ts.into_inner();

    if before_ts < 0 {
        return Err(
            MatrixError::invalid_param("Query parameter before_ts must be a positive integer")
                .into(),
        );
    }
    if before_ts < 30000000000 {
        return Err(MatrixError::invalid_param(
            "Query parameter before_ts you provided is from the year 1970. \
             Double check that you are providing a timestamp in milliseconds.",
        )
        .into());
    }

    let local_server = &config::get().server_name;
    let deleted = data::media::purge_remote_media_cache(local_server, before_ts)?;

    json_ok(PurgeMediaCacheResponse { deleted })
}
