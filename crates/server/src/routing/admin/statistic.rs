//! Admin Statistics API
//!
//! - GET /_synapse/admin/v1/statistics/users/media
//! - GET /_synapse/admin/v1/server_version

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::{JsonResult, data, json_ok};

pub fn router() -> Router {
    Router::new()
        .push(Router::with_path("v1/server_version").get(server_version))
        .push(Router::with_path("v1/statistics/users/media").get(user_media_statistics))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServerVersionResponse {
    pub server_version: String,
}

/// GET /_synapse/admin/v1/server_version
#[endpoint]
pub fn server_version() -> JsonResult<ServerVersionResponse> {
    json_ok(ServerVersionResponse {
        server_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserMediaStatisticsResponse {
    pub users: Vec<UserMediaStats>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserMediaStats {
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub displayname: Option<String>,
    pub media_count: i64,
    pub media_length: i64,
}

/// GET /_synapse/admin/v1/statistics/users/media
///
/// Get statistics about uploaded media by users
#[endpoint]
pub fn user_media_statistics(
    from: QueryParam<i64, false>,
    limit: QueryParam<i64, false>,
    order_by: QueryParam<String, false>,
    dir: QueryParam<String, false>,
    search_term: QueryParam<String, false>,
) -> JsonResult<UserMediaStatisticsResponse> {
    let from = from.into_inner().unwrap_or(0);
    let limit = limit.into_inner().unwrap_or(100);
    let order_by = order_by.into_inner();
    let dir = dir.into_inner();
    let search_term = search_term.into_inner();

    let (rows, total) = data::media::get_user_media_statistics(
        from,
        limit,
        search_term.as_deref(),
        order_by.as_deref(),
        dir.as_deref(),
    )?;

    let next_token = if from + limit < total {
        Some((from + limit).to_string())
    } else {
        None
    };

    let users = rows
        .into_iter()
        .map(|r| UserMediaStats {
            user_id: r.user_id,
            displayname: None,
            media_count: r.media_count,
            media_length: r.media_length,
        })
        .collect();

    json_ok(UserMediaStatisticsResponse {
        users,
        total,
        next_token,
    })
}
