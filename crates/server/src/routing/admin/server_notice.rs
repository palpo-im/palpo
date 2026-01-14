//! Admin Server Notice API
//!
//! - POST /_synapse/admin/v1/send_server_notice

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{JsonResult, MatrixError};

pub fn router() -> Router {
    Router::new().push(Router::with_path("v1/send_server_notice").post(send_server_notice))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendServerNoticeReqBody {
    pub user_id: String,
    pub content: serde_json::Value,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub state_key: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SendServerNoticeResponse {
    pub event_id: String,
}

/// POST /_synapse/admin/v1/send_server_notice
///
/// Send a server notice to a user
#[endpoint]
pub async fn send_server_notice(
    _body: JsonBody<SendServerNoticeReqBody>,
) -> JsonResult<SendServerNoticeResponse> {
    // Server notices are not enabled in this implementation
    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Server notices are not enabled on this server",
    )
    .into())
}
