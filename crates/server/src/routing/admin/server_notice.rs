//! Admin Server Notice API
//!
//! - POST /_synapse/admin/v1/send_server_notice
//!
//! Server notices allow admins to send messages directly to users via a special
//! "server notices room". This requires additional configuration:
//! - A dedicated server notices user (e.g., @_server:example.com)
//! - Automatic creation of notice rooms between the server user and target users
//!
//! Currently, this endpoint validates inputs but requires server notice infrastructure
//! to be fully implemented.

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::identifiers::*;
use crate::{JsonResult, MatrixError, config, data};

pub fn router() -> Router {
    Router::new().push(Router::with_path("v1/send_server_notice").post(send_server_notice))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendServerNoticeReqBody {
    /// The Matrix user ID to send the notice to
    pub user_id: String,
    /// The content of the message. Should include at least "msgtype" and "body" for m.room.message
    pub content: serde_json::Value,
    /// The event type (default: m.room.message)
    #[serde(default)]
    pub r#type: Option<String>,
    /// State key for state events
    #[serde(default)]
    pub state_key: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SendServerNoticeResponse {
    /// The event ID of the sent notice
    pub event_id: String,
}

/// POST /_synapse/admin/v1/send_server_notice
///
/// Send a server notice to a user.
///
/// Server notices are special messages sent from the homeserver to users,
/// typically used for important system notifications.
///
/// This endpoint requires server notices infrastructure to be configured.
#[endpoint(operation_id = "send_server_notice")]
pub async fn send_server_notice(
    body: JsonBody<SendServerNoticeReqBody>,
) -> JsonResult<SendServerNoticeResponse> {
    let body = body.into_inner();

    // Validate user_id format
    let user_id = UserId::parse(&body.user_id)
        .map_err(|_| MatrixError::invalid_param("Invalid user_id format"))?;

    // Only local users can receive server notices
    if *user_id.server_name() != *config::get().server_name {
        return Err(
            MatrixError::invalid_param("Server notices can only be sent to local users").into(),
        );
    }

    // Check if user exists
    if !data::user::user_exists(&user_id)? {
        return Err(MatrixError::not_found("User not found").into());
    }

    // Validate content has required fields for m.room.message
    let event_type = body.r#type.as_deref().unwrap_or("m.room.message");
    if event_type == "m.room.message" {
        if !body.content.is_object() {
            return Err(MatrixError::invalid_param("content must be a JSON object").into());
        }
        let content = body.content.as_object().unwrap();
        if !content.contains_key("msgtype") {
            return Err(MatrixError::invalid_param("content must contain 'msgtype' field").into());
        }
        if !content.contains_key("body") {
            return Err(MatrixError::invalid_param("content must contain 'body' field").into());
        }
    }

    // Server notices infrastructure is not yet fully implemented
    // Full implementation requires:
    // 1. A config section for the server notices user ID
    // 2. Creating/finding a notice room between the server user and target user
    // 3. Sending the event in that room
    //
    // For now, return a clear error message
    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Server notices are not enabled on this server. \
         This feature requires additional configuration of a server notices user \
         and automatic room creation infrastructure.",
    )
    .into())
}
