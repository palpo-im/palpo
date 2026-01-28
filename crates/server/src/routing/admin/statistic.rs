//! Admin Statistics API
//!
//! - GET /_synapse/admin/v1/statistics/users/media
//! - GET /_synapse/admin/v1/server_version

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::{JsonResult, MatrixError, json_ok};

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
    _order_by: QueryParam<String, false>,
    _dir: QueryParam<String, false>,
    _search_term: QueryParam<String, false>,
) -> JsonResult<UserMediaStatisticsResponse> {
    let _from = from.into_inner().unwrap_or(0);
    let _limit = limit.into_inner().unwrap_or(100);

    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "User media statistics are not enabled on this server",
    )
    .into())
}
