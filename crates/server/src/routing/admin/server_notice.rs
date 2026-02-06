//! Admin Server Notice API
//!
//! - POST /_synapse/admin/v1/send_server_notice
//!
//! Server notices allow admins to send messages directly to users via a special
//! "server notices room". The endpoint finds a shared room between the server
//! user and the target user, then sends the event there.

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::events::room::message::RoomMessageEventContent;
use crate::core::identifiers::*;
use crate::room::timeline;
use crate::{JsonResult, MatrixError, PduBuilder, config, data, json_ok};

pub fn router() -> Router {
    Router::new().push(Router::with_path("v1/send_server_notice").post(send_server_notice))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendServerNoticeReqBody {
    /// The Matrix user ID to send the notice to
    pub user_id: String,
    /// The content of the message
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
/// Finds a shared room between the server user and target user, then sends the event.
#[endpoint(operation_id = "send_server_notice")]
pub async fn send_server_notice(
    body: JsonBody<SendServerNoticeReqBody>,
) -> JsonResult<SendServerNoticeResponse> {
    let body = body.into_inner();

    let user_id = UserId::parse(&body.user_id)
        .map_err(|_| MatrixError::invalid_param("Invalid user_id format"))?;

    if *user_id.server_name() != *config::get().server_name {
        return Err(
            MatrixError::invalid_param("Server notices can only be sent to local users").into(),
        );
    }

    if !data::user::user_exists(&user_id)? {
        return Err(MatrixError::not_found("User not found").into());
    }

    let event_type = body.r#type.as_deref().unwrap_or("m.room.message");

    // Find an existing shared room between the server user and the target
    let server_user = crate::config::server_user_id();
    let rooms = crate::data::user::joined_rooms(&user_id)?;
    let mut notice_room = None;
    for room_id in &rooms {
        if crate::room::user::is_joined(server_user, room_id)? {
            notice_room = Some(room_id.clone());
            break;
        }
    }

    let room_id = notice_room.ok_or_else(|| {
        MatrixError::not_found(
            "No shared room found between server user and target user. \
             Server notices require a room that both the server user and \
             target user have joined.",
        )
    })?;

    let state_lock = crate::room::lock_state(&room_id).await;
    let room_version = crate::room::get_version(&room_id)?;

    let pdu = if event_type == "m.room.message" {
        let content: RoomMessageEventContent =
            serde_json::from_value(body.content)
                .map_err(|e| MatrixError::invalid_param(format!("Invalid content: {e}")))?;
        timeline::build_and_append_pdu(
            PduBuilder::timeline(&content),
            server_user,
            &room_id,
            &room_version,
            &state_lock,
        )
        .await?
    } else {
        let pdu_builder = PduBuilder {
            event_type: event_type.into(),
            content: serde_json::value::to_raw_value(&body.content)?,
            unsigned: Default::default(),
            state_key: body.state_key,
            redacts: None,
            timestamp: None,
        };
        timeline::build_and_append_pdu(
            pdu_builder,
            server_user,
            &room_id,
            &room_version,
            &state_lock,
        )
        .await?
    };

    json_ok(SendServerNoticeResponse {
        event_id: pdu.event_id.to_string(),
    })
}
