use std::collections::BTreeMap;
use std::time::Duration;

use diesel::prelude::*;
use tokio::sync::RwLock;

use crate::core::events::direct::DirectEventContent;
use crate::core::events::room::create::RoomCreateEventContent;
use crate::core::events::room::member::MembershipState;
use crate::core::events::{
    AnyStrippedStateEvent, GlobalAccountDataEventType, RoomAccountDataEventType, StateEventType,
};
use crate::core::identifiers::*;
use crate::core::serde::{
    CanonicalJsonObject, CanonicalJsonValue, JsonValue, RawJson, RawJsonValue,
};
use crate::core::{UnixMillis, federation};
use crate::data::connect;
use crate::data::room::NewDbRoomUser;
use crate::data::schema::*;
use crate::room::state::ensure_field;
use crate::{AppError, AppResult, MatrixError, SigningKeys, room};

mod banned;
mod forget;
mod invite;
mod join;
mod knock;
mod leave;
pub use banned::*;
pub use forget::*;
pub use invite::*;
pub use join::*;
pub use knock::*;
pub use leave::*;

async fn validate_and_add_event_id(
    pdu: &RawJsonValue,
    room_version: &RoomVersionId,
    _pub_key_map: &RwLock<BTreeMap<String, SigningKeys>>,
) -> AppResult<(OwnedEventId, CanonicalJsonObject)> {
    let (event_id, mut value) = crate::event::gen_event_id_canonical_json(pdu, room_version)?;

    // TODO
    // let back_off = |id| match crate::BAD_EVENT_RATE_LIMITER.write().unwrap().entry(id) {
    //     Entry::Vacant(e) => {
    //         e.insert((Instant::now(), 1));
    //     }
    //     Entry::Occupied(mut e) => *e.get_mut() = (Instant::now(), e.get().1 + 1),
    // };

    if let Some((time, tries)) = crate::BAD_EVENT_RATE_LIMITER.read().unwrap().get(&event_id) {
        // Exponential backoff
        let mut min_elapsed_duration = Duration::from_secs(30) * (*tries) * (*tries);
        if min_elapsed_duration > Duration::from_secs(60 * 60 * 24) {
            min_elapsed_duration = Duration::from_secs(60 * 60 * 24);
        }

        if time.elapsed() < min_elapsed_duration {
            debug!("backing off from {}", event_id);
            return Err(AppError::public("bad event, still backing off"));
        }
    }

    let origin_server_ts = value.get("origin_server_ts").ok_or_else(|| {
        error!("invalid pdu, no origin_server_ts field");
        MatrixError::missing_param("invalid pdu, no origin_server_ts field")
    })?;

    let _origin_server_ts: UnixMillis = {
        let ts = origin_server_ts
            .as_integer()
            .ok_or_else(|| MatrixError::invalid_param("origin_server_ts must be an integer"))?;

        UnixMillis(
            ts.try_into()
                .map_err(|_| MatrixError::invalid_param("time must be after the unix epoch"))?,
        )
    };

    // let unfiltered_keys = (*pub_key_map.read().await).clone();

    // let keys = crate::filter_keys_server_map(unfiltered_keys, origin_server_ts, room_version);

    // TODO
    // if let Err(e) = crate::core::signatures::verify_event(&keys, &value, room_version) {
    //     warn!("Event {} failed verification {:?} {}", event_id, pdu, e);
    //     back_off(event_id);
    //     return Err(AppError::public("Event failed verification."));
    // }

    value.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.as_str().to_owned()),
    );

    Ok((event_id, value))
}

/// Update current membership data.
#[tracing::instrument(skip(last_state))]
pub fn update_membership(
    event_id: &EventId,
    event_sn: i64,
    room_id: &RoomId,
    user_id: &UserId,
    membership: MembershipState,
    sender_id: &UserId,
    last_state: Option<Vec<RawJson<AnyStrippedStateEvent>>>,
) -> AppResult<()> {
    let conf = crate::config::get();
    // Keep track what remote users exist by adding them as "deactivated" users
    if user_id.server_name() != conf.server_name && !crate::data::user::user_exists(user_id)? {
        crate::user::create_user(user_id, None)?;
        // TODO: display_name, avatar url
    }

    let state_data = if let Some(last_state) = &last_state {
        Some(serde_json::to_value(last_state)?)
    } else {
        None
    };

    match &membership {
        MembershipState::Join => {
            // Check if the user never joined this room
            if !(room::user::is_joined(user_id, room_id)? || room::user::is_left(user_id, room_id)?)
            {
                // Check if the room has a predecessor
                if let Ok(Some(predecessor)) = room::get_state_content::<RoomCreateEventContent>(
                    room_id,
                    &StateEventType::RoomCreate,
                    "",
                    None,
                )
                .map(|c| c.predecessor)
                {
                    // Copy user settings from predecessor to the current room:
                    // - Push rules
                    //
                    // TODO: finish this once push rules are implemented.
                    //
                    // let mut push_rules_event_content: PushRulesEvent = account_data
                    //     .get(
                    //         None,
                    //         user_id,
                    //         EventType::PushRules,
                    //     )?;
                    //
                    // NOTE: find where `predecessor.room_id` match
                    //       and update to `room_id`.
                    //
                    // account_data
                    //     .update(
                    //         None,
                    //         user_id,
                    //         EventType::PushRules,
                    //         &push_rules_event_content,
                    //         globals,
                    //     )
                    //     .ok();

                    // Copy old tags to new room
                    if let Some(tag_event_content) = crate::data::user::get_room_data::<JsonValue>(
                        user_id,
                        &predecessor.room_id,
                        &RoomAccountDataEventType::Tag.to_string(),
                    )? {
                        crate::data::user::set_data(
                            user_id,
                            Some(room_id.to_owned()),
                            &RoomAccountDataEventType::Tag.to_string(),
                            tag_event_content,
                        )
                        .ok();
                    };

                    // Copy direct chat flag
                    if let Ok(mut direct_event_content) =
                        crate::data::user::get_data::<DirectEventContent>(
                            user_id,
                            None,
                            &GlobalAccountDataEventType::Direct.to_string(),
                        )
                    {
                        let mut room_ids_updated = false;

                        for room_ids in direct_event_content.0.values_mut() {
                            if room_ids.iter().any(|r| r == &predecessor.room_id) {
                                room_ids.push(room_id.to_owned());
                                room_ids_updated = true;
                            }
                        }

                        if room_ids_updated {
                            crate::data::user::set_data(
                                user_id,
                                None,
                                &GlobalAccountDataEventType::Direct.to_string(),
                                serde_json::to_value(&direct_event_content)?,
                            )?;
                        }
                    };
                }
            }
            connect()?.transaction::<_, AppError, _>(|conn| {
                // let forgotten = room_users::table
                //     .filter(room_users::room_id.eq(room_id))
                //     .filter(room_users::user_id.eq(user_id))
                //     .select(room_users::forgotten)
                //     .first::<bool>(conn)
                //     .optional()?
                //     .unwrap_or_default();
                diesel::delete(
                    room_users::table
                        .filter(room_users::room_id.eq(room_id))
                        .filter(room_users::user_id.eq(user_id)),
                )
                .execute(conn)?;
                diesel::insert_into(room_users::table)
                    .values(&NewDbRoomUser {
                        room_id: room_id.to_owned(),
                        room_server_id: room_id.server_name().ok().map(|v| v.to_owned()),
                        user_id: user_id.to_owned(),
                        user_server_id: user_id.server_name().to_owned(),
                        event_id: event_id.to_owned(),
                        event_sn,
                        sender_id: sender_id.to_owned(),
                        membership: membership.to_string(),
                        forgotten: false,
                        display_name: None,
                        avatar_url: None,
                        state_data,
                        created_at: UnixMillis::now(),
                    })
                    .execute(conn)?;
                Ok(())
            })?;
        }
        MembershipState::Invite | MembershipState::Knock => {
            // We want to know if the sender is ignored by the receiver
            if crate::user::user_is_ignored(sender_id, user_id) {
                return Ok(());
            }
            if let Some(last_state) = &last_state {
                for event in last_state {
                    if let Ok(event) = event.deserialize() {
                        let _ = ensure_field(&event.event_type(), event.state_key());
                    }
                }
            }
            let _ = ensure_field(&StateEventType::RoomMember, user_id.as_str());
            connect()?.transaction::<_, AppError, _>(|conn| {
                // let forgotten = room_users::table
                //     .filter(room_users::room_id.eq(room_id))
                //     .filter(room_users::user_id.eq(user_id))
                //     .select(room_users::forgotten)
                //     .first::<bool>(conn)
                //     .optional()?
                //     .unwrap_or_default();
                diesel::delete(
                    room_users::table
                        .filter(room_users::room_id.eq(room_id))
                        .filter(room_users::user_id.eq(user_id)),
                )
                .execute(conn)?;
                diesel::insert_into(room_users::table)
                    .values(&NewDbRoomUser {
                        room_id: room_id.to_owned(),
                        room_server_id: room_id.server_name().ok().map(|v| v.to_owned()),
                        user_id: user_id.to_owned(),
                        user_server_id: user_id.server_name().to_owned(),
                        event_id: event_id.to_owned(),
                        event_sn,
                        sender_id: sender_id.to_owned(),
                        membership: membership.to_string(),
                        forgotten: false,
                        display_name: None,
                        avatar_url: None,
                        state_data,
                        created_at: UnixMillis::now(),
                    })
                    .execute(conn)?;
                Ok(())
            })?;
        }
        MembershipState::Leave | MembershipState::Ban => {
            connect()?.transaction::<_, AppError, _>(|conn| {
                // let forgotten = room_users::table
                //     .filter(room_users::room_id.eq(room_id))
                //     .filter(room_users::user_id.eq(user_id))
                //     .select(room_users::forgotten)
                //     .first::<bool>(conn)
                //     .optional()?
                //     .unwrap_or_default();
                diesel::delete(
                    room_users::table
                        .filter(room_users::room_id.eq(room_id))
                        .filter(room_users::user_id.eq(user_id)),
                )
                .execute(conn)?;
                diesel::insert_into(room_users::table)
                    .values(&NewDbRoomUser {
                        room_id: room_id.to_owned(),
                        room_server_id: room_id.server_name().ok().map(|v| v.to_owned()),
                        user_id: user_id.to_owned(),
                        user_server_id: user_id.server_name().to_owned(),
                        event_id: event_id.to_owned(),
                        event_sn,
                        sender_id: sender_id.to_owned(),
                        membership: membership.to_string(),
                        forgotten: false,
                        display_name: None,
                        avatar_url: None,
                        state_data,
                        created_at: UnixMillis::now(),
                    })
                    .execute(conn)?;
                Ok(())
            })?;
        }
        _ => {}
    }
    crate::room::update_joined_servers(room_id)?;
    crate::room::update_currents(room_id)?;
    Ok(())
}
