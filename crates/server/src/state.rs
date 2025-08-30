use crate::core::events::room::canonical_alias::RoomCanonicalAliasEventContent;
use crate::core::events::room::history_visibility::{
    HistoryVisibility, RoomHistoryVisibilityEventContent,
};
use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::{AnyStateEventContent, StateEventType};
use crate::core::identifiers::*;
use crate::core::room::JoinRule;
use crate::core::serde::RawJson;
use crate::event::PduBuilder;
use crate::room::timeline;
use crate::{AppResult, IsRemoteOrLocal, MatrixError, config, room};

pub async fn send_state_event_for_key(
    user_id: &UserId,
    room_id: &RoomId,
    event_type: &StateEventType,
    json: RawJson<AnyStateEventContent>,
    state_key: String,
) -> AppResult<OwnedEventId> {
    allowed_to_send_state_event(room_id, event_type, &state_key, &json)?;
    let pdu = timeline::build_and_append_pdu(
        PduBuilder {
            event_type: event_type.to_string().into(),
            content: serde_json::from_value(serde_json::to_value(json)?)?,
            state_key: Some(state_key),
            ..Default::default()
        },
        user_id,
        room_id,
        &room::lock_state(room_id).await,
    )
    .await?;

    Ok(pdu.pdu.event_id)
}

fn allowed_to_send_state_event(
    room_id: &RoomId,
    event_type: &StateEventType,
    state_key: &str,
    json: &RawJson<AnyStateEventContent>,
) -> AppResult<()> {
    let conf = config::get();
    match event_type {
        StateEventType::RoomCreate => {
            return Err(MatrixError::bad_json(
                "You cannot update m.room.create after a room has been created.",
            )
            .into());
        }
        // Forbid m.room.encryption if encryption is disabled
        StateEventType::RoomEncryption => {
            if !conf.allow_encryption {
                return Err(MatrixError::forbidden(
                    "Encryption is disabled on this homeserver.",
                    None,
                )
                .into());
            }
        }
        // admin room is a sensitive room, it should not ever be made public
        StateEventType::RoomJoinRules => {
            if crate::room::is_admin_room(room_id)?
                && let Ok(join_rule) =
                    serde_json::from_str::<RoomJoinRulesEventContent>(json.inner().get())
                && join_rule.join_rule == JoinRule::Public
            {
                return Err(MatrixError::forbidden(
                    "Admin room is a sensitive room, it cannot be made public",
                    None,
                )
                .into());
            }
        }
        // admin room is a sensitive room, it should not ever be made world readable
        StateEventType::RoomHistoryVisibility => {
            if let Ok(visibility_content) =
                serde_json::from_str::<RoomHistoryVisibilityEventContent>(json.inner().get())
                && crate::room::is_admin_room(room_id)?
                && visibility_content.history_visibility == HistoryVisibility::WorldReadable
            {
                return Err(MatrixError::forbidden(
                    "Admin room is a sensitive room, it cannot be made world readable \
							 (public room history).",
                    None,
                )
                .into());
            }
        }
        StateEventType::RoomCanonicalAlias => {
            if let Ok(canonical_alias) =
                serde_json::from_str::<RoomCanonicalAliasEventContent>(json.inner().get())
            {
                let mut aliases = canonical_alias.alt_aliases.clone();

                if let Some(alias) = canonical_alias.alias {
                    aliases.push(alias);
                }

                for alias in aliases {
                    if !alias.server_name().is_local() {
                        return Err(MatrixError::forbidden(
                            "Canonical_alias must be for this server.",
                            None,
                        )
                        .into());
                    }

                    if !crate::room::resolve_local_alias(&alias).is_ok_and(|room| room == room_id)
                    // Make sure it's the right room
                    {
                        return Err(MatrixError::bad_alias(
                            "You are only allowed to send canonical_alias events when its \
							 aliases already exist",
                        )
                        .into());
                    }
                }
            } else {
                return Err(MatrixError::invalid_param("Invalid aliases or alt_aliases").into());
            }
        }
        StateEventType::RoomMember => {
            let Ok(membership_content) =
                serde_json::from_str::<RoomMemberEventContent>(json.inner().get())
            else {
                return Err(MatrixError::bad_json(
                    "Membership content must have a valid JSON body with at least a valid \
					 membership state.",
                )
                .into());
            };

            let Ok(state_key) = UserId::parse(state_key) else {
                return Err(MatrixError::bad_json(
                    "Membership event has invalid or non-existent state key",
                )
                .into());
            };

            if let Some(authorising_user) = membership_content.join_authorized_via_users_server {
                if membership_content.membership != MembershipState::Join {
                    return Err(MatrixError::bad_json(
                        "join_authorised_via_users_server is only for member joins",
                    )
                    .into());
                }

                if crate::room::user::is_joined(&state_key, room_id)? {
                    return Err(MatrixError::invalid_param(
                        "{state_key} is already joined, an authorising user is not required.",
                    )
                    .into());
                }

                if !&authorising_user.is_local() {
                    return Err(MatrixError::invalid_param(
                        "Authorising user {authorising_user} does not belong to this homeserver",
                    )
                    .into());
                }

                if !crate::room::user::is_joined(&authorising_user, room_id)? {
                    return Err(MatrixError::invalid_param(
                        "Authorising user {authorising_user} is not in the room, they cannot \
						 authorise the join.",
                    )
                    .into());
                }
            }
        }
        _ => (),
    }

    Ok(())
}
