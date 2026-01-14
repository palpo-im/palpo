//! Admin Media API
//!
//! - GET /_synapse/admin/v1/media/{server_name}/{media_id}
//! - DELETE /_synapse/admin/v1/media/{server_name}/{media_id}
//! - GET /_synapse/admin/v1/room/{room_id}/media
//! - GET /_synapse/admin/v1/users/{user_id}/media
//! - POST /_synapse/admin/v1/purge_media_cache

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::core::identifiers::*;
use crate::{JsonResult, MatrixError};

pub fn router() -> Router {
    Router::new()
        .push(
            Router::with_path("v1/media/{server_name}/{media_id}")
                .get(get_media_info)
                .delete(delete_media),
        )
        .push(Router::with_path("v1/room/{room_id}/media").get(list_media_in_room))
        .push(Router::with_path("v1/users/{user_id}/media").get(list_user_media))
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_access_ts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarantined_by: Option<String>,
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
    pub next_token: Option<String>,
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

/// GET /_synapse/admin/v1/media/{server_name}/{media_id}
///
/// Get information about a piece of media
#[endpoint]
pub fn get_media_info(
    server_name: PathParam<OwnedServerName>,
    media_id: PathParam<String>,
) -> JsonResult<MediaInfoResponse> {
    let _server_name = server_name.into_inner();
    let _media_id = media_id.into_inner();

    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Media admin endpoints are not enabled on this server",
    )
    .into())
}

/// DELETE /_synapse/admin/v1/media/{server_name}/{media_id}
///
/// Delete a piece of media
#[endpoint]
pub fn delete_media(
    server_name: PathParam<OwnedServerName>,
    media_id: PathParam<String>,
) -> JsonResult<DeleteMediaResponse> {
    let _server_name = server_name.into_inner();
    let _media_id = media_id.into_inner();

    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Media admin endpoints are not enabled on this server",
    )
    .into())
}

/// GET /_synapse/admin/v1/room/{room_id}/media
///
/// List all media in a room
#[endpoint]
pub fn list_media_in_room(room_id: PathParam<OwnedRoomId>) -> JsonResult<RoomMediaResponse> {
    let _room_id = room_id.into_inner();

    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Media admin endpoints are not enabled on this server",
    )
    .into())
}

/// GET /_synapse/admin/v1/users/{user_id}/media
///
/// List all media uploaded by a user
#[endpoint]
pub fn list_user_media(
    user_id: PathParam<OwnedUserId>,
    from: QueryParam<i64, false>,
    limit: QueryParam<i64, false>,
) -> JsonResult<UserMediaResponse> {
    let _user_id = user_id.into_inner();
    let _from = from.into_inner().unwrap_or(0);
    let _limit = limit.into_inner().unwrap_or(100);

    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Media admin endpoints are not enabled on this server",
    )
    .into())
}

/// POST /_synapse/admin/v1/purge_media_cache
///
/// Purge old cached remote media
#[endpoint]
pub fn purge_media_cache(before_ts: QueryParam<i64, true>) -> JsonResult<PurgeMediaCacheResponse> {
    let _before_ts = before_ts.into_inner();

    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Media admin endpoints are not enabled on this server",
    )
    .into())
}
