//! Admin Server Notice API
//!
//! - POST /_synapse/admin/v1/send_server_notice
//!
//! Server notices allow admins to send messages directly to users via a special
//! "server notices room". The endpoint finds or creates a shared room between
//! the server user and the target user, then sends the event there.

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::value::to_raw_value;

use crate::core::events::room::create::RoomCreateEventContent;
use crate::core::events::room::guest_access::{GuestAccess, RoomGuestAccessEventContent};
use crate::core::events::room::history_visibility::{
    HistoryVisibility, RoomHistoryVisibilityEventContent,
};
use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::room::message::RoomMessageEventContent;
use crate::core::events::room::name::RoomNameEventContent;
use crate::core::events::room::power_levels::RoomPowerLevelsEventContent;
use crate::core::events::TimelineEventType;
use crate::core::identifiers::*;
use crate::core::room_version_rules::RoomIdFormatVersion;
use crate::core::room::JoinRule;
use crate::room::timeline;
use crate::{JsonResult, MatrixError, PduBuilder, config, data, json_ok, room};

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

    // Find an existing shared room between the server user and the target,
    // or create one if none exists.
    let server_user = crate::config::server_user_id();
    let rooms = crate::data::user::joined_rooms(&user_id)?;
    let mut notice_room = None;
    for room_id in &rooms {
        if crate::room::user::is_joined(server_user, room_id)? {
            notice_room = Some(room_id.clone());
            break;
        }
    }

    let room_id = match notice_room {
        Some(id) => id,
        None => create_notice_room(server_user, &user_id).await?,
    };

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

/// Create a private DM room between the server user and a target user for server notices.
async fn create_notice_room(
    server_user: &UserId,
    target_user: &UserId,
) -> crate::AppResult<OwnedRoomId> {
    let conf = config::get();
    let room_version = conf.default_room_version.clone();
    let version_rules = room::get_version_rules(&room_version)?;

    let mut create_content = match room_version {
        RoomVersionId::V11 => RoomCreateEventContent::new_v11(),
        RoomVersionId::V12 => RoomCreateEventContent::new_v12(),
        _ => RoomCreateEventContent::new_v1(server_user.to_owned()),
    };
    create_content.room_version = room_version.clone();

    let (room_id, state_lock) = match version_rules.room_id_format {
        RoomIdFormatVersion::V1 => {
            let room_id = RoomId::new_v1(&conf.server_name);
            let state_lock = room::lock_state(&room_id).await;
            room::ensure_room(&room_id, &room_version)?;

            // 1. Room create event
            timeline::build_and_append_pdu(
                PduBuilder {
                    event_type: TimelineEventType::RoomCreate,
                    content: to_raw_value(&create_content)?,
                    state_key: Some(String::new()),
                    ..Default::default()
                },
                server_user,
                &room_id,
                &room_version,
                &state_lock,
            )
            .await?;

            (room_id, state_lock)
        }
        RoomIdFormatVersion::V2 => {
            let temp_room_id =
                OwnedRoomId::try_from("!placehold").expect("placeholder room id is valid");
            let temp_lock = room::lock_state(&temp_room_id).await;
            room::ensure_room(&temp_room_id, &room_version)?;

            // 1. Room create event, using a placeholder room_id
            let create_event = timeline::build_and_append_pdu(
                PduBuilder {
                    event_type: TimelineEventType::RoomCreate,
                    content: to_raw_value(&create_content)?,
                    state_key: Some(String::new()),
                    ..Default::default()
                },
                server_user,
                &temp_room_id,
                &room_version,
                &temp_lock,
            )
            .await?;
            drop(temp_lock);

            let state_lock = room::lock_state(&create_event.room_id).await;
            (create_event.room_id.clone(), state_lock)
        }
    };

    // 2. Server user joins
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&RoomMemberEventContent {
                membership: MembershipState::Join,
                display_name: data::user::display_name(server_user).ok().flatten(),
                avatar_url: data::user::avatar_url(server_user).ok().flatten(),
                is_direct: Some(true),
                third_party_invite: None,
                blurhash: None,
                reason: None,
                join_authorized_via_users_server: None,
                extra_data: Default::default(),
            })?,
            state_key: Some(server_user.to_string()),
            ..Default::default()
        },
        server_user,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 3. Power levels — server user gets admin power
    let mut power_levels_content = RoomPowerLevelsEventContent::new(&version_rules.authorization);
    power_levels_content
        .users
        .insert(server_user.to_owned(), 100);
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomPowerLevels,
            content: to_raw_value(&power_levels_content)?,
            state_key: Some(String::new()),
            ..Default::default()
        },
        server_user,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 4. Join rules — invite only
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomJoinRules,
            content: to_raw_value(&RoomJoinRulesEventContent::new(JoinRule::Invite))?,
            state_key: Some(String::new()),
            ..Default::default()
        },
        server_user,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 5. History visibility — joined members only
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomHistoryVisibility,
            content: to_raw_value(&RoomHistoryVisibilityEventContent::new(
                HistoryVisibility::Joined,
            ))?,
            state_key: Some(String::new()),
            ..Default::default()
        },
        server_user,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 6. Guest access — forbidden
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomGuestAccess,
            content: to_raw_value(&RoomGuestAccessEventContent::new(GuestAccess::Forbidden))?,
            state_key: Some(String::new()),
            ..Default::default()
        },
        server_user,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 7. Room name
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomName,
            content: to_raw_value(&RoomNameEventContent::new("Server Notices".to_owned()))?,
            state_key: Some(String::new()),
            ..Default::default()
        },
        server_user,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 8. Invite target user
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&RoomMemberEventContent {
                membership: MembershipState::Invite,
                display_name: None,
                avatar_url: None,
                is_direct: Some(true),
                third_party_invite: None,
                blurhash: None,
                reason: None,
                join_authorized_via_users_server: None,
                extra_data: Default::default(),
            })?,
            state_key: Some(target_user.to_string()),
            ..Default::default()
        },
        server_user,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 9. Auto-join target user
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&RoomMemberEventContent {
                membership: MembershipState::Join,
                display_name: data::user::display_name(target_user).ok().flatten(),
                avatar_url: data::user::avatar_url(target_user).ok().flatten(),
                is_direct: Some(true),
                third_party_invite: None,
                blurhash: None,
                reason: None,
                join_authorized_via_users_server: None,
                extra_data: Default::default(),
            })?,
            state_key: Some(target_user.to_string()),
            ..Default::default()
        },
        target_user,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    drop(state_lock);
    Ok(room_id)
}
