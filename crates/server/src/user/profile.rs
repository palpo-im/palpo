//! Shared profile mutations for client, administrator and MAS routes.
//!
//! The global profile commits first. Each room is then independently published;
//! errors are returned to the caller, and retrying the same mutation reconciles
//! every joined room without creating duplicate member events. Matching events
//! are requeued for federation so a previous enqueue failure remains retryable.
//! Success means room propagation and the final presence refresh completed.
use serde_json::value::to_raw_value;

use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::{OwnedMxcUri, UserId};
use crate::{AppResult, MatrixError, PduBuilder, data, room};

#[derive(Clone, Copy)]
enum Field {
    DisplayName,
    Avatar,
}

pub async fn set_display_name(user_id: &UserId, value: Option<&str>) -> AppResult<()> {
    data::user::set_global_display_name(user_id, value).await?;
    propagate(user_id, Field::DisplayName).await
}

pub async fn set_avatar(
    user_id: &UserId,
    avatar: Option<&crate::core::MxcUri>,
    blurhash: Option<&str>,
) -> AppResult<()> {
    data::user::set_global_avatar_and_blurhash(user_id, avatar, blurhash).await?;
    propagate(user_id, Field::Avatar).await
}

/// Empty administrator avatar values clear the URI and its stale blurhash.
pub async fn set_admin_avatar(user_id: &UserId, value: &str) -> AppResult<()> {
    let avatar: Option<OwnedMxcUri> = (!value.is_empty()).then(|| value.into());
    set_avatar(user_id, avatar.as_deref(), None).await
}

async fn propagate(user_id: &UserId, field: Field) -> AppResult<()> {
    let mut joined = data::user::joined_rooms(user_id).await?;
    joined.sort();
    for room_id in joined {
        // Membership can change while waiting. Read both the current member and
        // the latest global value only after acquiring the room's write lock.
        let state_lock = room::lock_state(&room_id).await;
        let current = room::get_state(
            &room_id,
            &StateEventType::RoomMember,
            user_id.as_str(),
            None,
        )
        .await?;
        let mut member = current.get_content::<RoomMemberEventContent>()?;
        if member.membership != MembershipState::Join {
            continue;
        }
        let profile = data::user::get_profile(user_id, None)
            .await?
            .ok_or_else(|| MatrixError::not_found("Profile was not found."))?;
        let changed = match field {
            Field::DisplayName => {
                let changed = member.display_name != profile.display_name;
                member.display_name = profile.display_name;
                changed
            }
            Field::Avatar => {
                let changed =
                    member.avatar_url != profile.avatar_url || member.blurhash != profile.blurhash;
                member.avatar_url = profile.avatar_url;
                member.blurhash = profile.blurhash;
                changed
            }
        };
        if !changed {
            // State publication precedes federation queueing. An earlier call
            // may have committed this member but failed to enqueue its PDU.
            room::timeline::deliver_local_pdu(&room_id, &current.event_id).await?;
            continue;
        }
        room::timeline::build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomMember,
                content: to_raw_value(&member)?,
                state_key: Some(user_id.to_string()),
                ..Default::default()
            },
            user_id,
            &room_id,
            &room::get_version(&room_id).await?,
            &state_lock,
        )
        .await?;
    }
    data::user::refresh_presence_for_profile(user_id).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
