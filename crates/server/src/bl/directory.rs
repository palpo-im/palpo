use crate::core::directory::{PublicRoomFilter, PublicRoomJoinRule, PublicRoomsChunk, PublicRoomsResBody, RoomNetwork};
use crate::core::events::room::avatar::RoomAvatarEventContent;
use crate::core::events::room::canonical_alias::RoomCanonicalAliasEventContent;
use crate::core::events::room::create::RoomCreateEventContent;
use crate::core::events::room::guest_access::{GuestAccess, RoomGuestAccessEventContent};
use crate::core::events::room::history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent};
use crate::core::events::room::join_rules::{JoinRule, RoomJoinRulesEventContent};
use crate::core::events::room::topic::RoomTopicEventContent;
use crate::core::events::StateEventType;
use crate::core::federation::directory::{public_rooms_request, PublicRoomsReqBody};
use crate::core::ServerName;
use crate::{AppError, AppResult, MatrixError};

pub async fn get_public_rooms(
    server: Option<&ServerName>,
    limit: Option<usize>,
    since: Option<&str>,
    filter: &PublicRoomFilter,
    network: &RoomNetwork,
) -> AppResult<PublicRoomsResBody> {
    if let Some(other_server) = server.filter(|server| *server != crate::server_name().as_str()) {
        let body = public_rooms_request(
            other_server,
            PublicRoomsReqBody {
                limit,
                since: since.map(ToOwned::to_owned),
                filter: PublicRoomFilter {
                    generic_search_term: filter.generic_search_term.clone(),
                    room_types: filter.room_types.clone(),
                },
                room_network: RoomNetwork::Matrix,
            },
        )?
        .send()
        .await?;

        Ok(body)
    } else {
        get_local_public_rooms(limit, since, filter, network)
    }
}

fn get_local_public_rooms(
    limit: Option<usize>,
    since: Option<&str>,
    filter: &PublicRoomFilter,
    _network: &RoomNetwork,
) -> AppResult<PublicRoomsResBody> {
    let limit = limit.unwrap_or(10);
    let mut num_since = 0_u64;

    if let Some(s) = &since {
        let mut characters = s.chars();
        let backwards = match characters.next() {
            Some('n') => false,
            Some('p') => true,
            _ => return Err(MatrixError::invalid_param("Invalid `since` token").into()),
        };

        num_since = characters
            .collect::<String>()
            .parse()
            .map_err(|_| MatrixError::invalid_param("Invalid `since` token."))?;

        if backwards {
            num_since = num_since.saturating_sub(limit as u64);
        }
    }

    let mut all_rooms: Vec<_> = crate::room::public_room_ids()?
        .into_iter()
        .map(|room_id| {
            let chunk = PublicRoomsChunk {
                canonical_alias: crate::room::state::get_state(
                    &room_id,
                    &StateEventType::RoomCanonicalAlias,
                    "",
                    None,
                )?
                .map_or(Ok(None), |s| {
                    serde_json::from_str(s.content.get())
                        .map(|c: RoomCanonicalAliasEventContent| c.alias)
                        .map_err(|_| AppError::public("Invalid canonical alias event in database."))
                })?,
                name: crate::room::state::get_name(&room_id, None)?,
                num_joined_members: crate::room::joined_member_count(&room_id)
                    .unwrap_or_else(|_| {
                        warn!("Room {} has no member count", room_id);
                        0
                    })
                    .try_into()
                    .expect("user count should not be that big"),
                topic: crate::room::state::get_state(&room_id, &StateEventType::RoomTopic, "", None)?.map_or(
                    Ok(None),
                    |s| {
                        serde_json::from_str(s.content.get())
                            .map(|c: RoomTopicEventContent| Some(c.topic))
                            .map_err(|_| {
                                error!("Invalid room topic event in database for room {}", room_id);
                                AppError::public("Invalid room topic event in database.")
                            })
                    },
                )?,
                world_readable: crate::room::state::get_state(
                    &room_id,
                    &StateEventType::RoomHistoryVisibility,
                    "",
                    None,
                )?
                .map_or(Ok(false), |s| {
                    serde_json::from_str(s.content.get())
                        .map(|c: RoomHistoryVisibilityEventContent| {
                            c.history_visibility == HistoryVisibility::WorldReadable
                        })
                        .map_err(|_| AppError::public("Invalid room history visibility event in database."))
                })?,
                guest_can_join: crate::room::state::get_state(&room_id, &StateEventType::RoomGuestAccess, "", None)?
                    .map_or(Ok(false), |s| {
                        serde_json::from_str(s.content.get())
                            .map(|c: RoomGuestAccessEventContent| c.guest_access == GuestAccess::CanJoin)
                            .map_err(|_| AppError::public("Invalid room guest access event in database."))
                    })?,
                avatar_url: crate::room::state::get_state(&room_id, &StateEventType::RoomAvatar, "", None)?
                    .map(|s| {
                        serde_json::from_str(s.content.get())
                            .map(|c: RoomAvatarEventContent| c.url)
                            .map_err(|_| AppError::public("Invalid room avatar event in database."))
                    })
                    .transpose()?
                    // url is now an Option<String> so we must flatten
                    .flatten(),
                join_rule: crate::room::state::get_state(&room_id, &StateEventType::RoomJoinRules, "", None)?
                    .map(|s| {
                        serde_json::from_str(s.content.get())
                            .map(|c: RoomJoinRulesEventContent| match c.join_rule {
                                JoinRule::Public => Some(PublicRoomJoinRule::Public),
                                JoinRule::Knock => Some(PublicRoomJoinRule::Knock),
                                _ => None,
                            })
                            .map_err(|e| {
                                error!("Invalid room join rule event in database: {}", e);
                                AppError::public("Invalid room join rule event in database.")
                            })
                    })
                    .transpose()?
                    .flatten()
                    .ok_or_else(|| AppError::public("Missing room join rule event for room."))?,
                room_type: crate::room::state::get_state(&room_id, &StateEventType::RoomCreate, "", None)?
                    .map(|s| {
                        serde_json::from_str::<RoomCreateEventContent>(s.content.get()).map_err(|e| {
                            error!("Invalid room create event in database: {}", e);
                            AppError::public("Invalid room create event in database.")
                        })
                    })
                    .transpose()?
                    .and_then(|e| e.room_type),
                room_id,
            };
            Ok(chunk)
        })
        .filter_map(|r: AppResult<_>| r.ok()) // Filter out buggy rooms
        .filter(|chunk| {
            if let Some(query) = filter.generic_search_term.as_ref().map(|q| q.to_lowercase()) {
                if let Some(name) = &chunk.name {
                    if name.as_str().to_lowercase().contains(&query) {
                        return true;
                    }
                }

                if let Some(topic) = &chunk.topic {
                    if topic.to_lowercase().contains(&query) {
                        return true;
                    }
                }

                if let Some(canonical_alias) = &chunk.canonical_alias {
                    if canonical_alias.as_str().to_lowercase().contains(&query) {
                        return true;
                    }
                }

                false
            } else {
                // No search term
                true
            }
        })
        // We need to collect all, so we can sort by member count
        .collect();

    all_rooms.sort_by(|l, r| r.num_joined_members.cmp(&l.num_joined_members));

    let total_room_count_estimate = (all_rooms.len() as u32).into();

    let chunk: Vec<_> = all_rooms
        .into_iter()
        .skip(num_since as usize)
        .take(limit as usize)
        .collect();

    let prev_batch = if num_since == 0 {
        None
    } else {
        Some(format!("p{num_since}"))
    };

    let next_batch = if chunk.len() < limit as usize {
        None
    } else {
        Some(format!("n{}", num_since + limit as u64))
    };

    Ok(PublicRoomsResBody {
        chunk,
        prev_batch,
        next_batch,
        total_room_count_estimate: Some(total_room_count_estimate),
    })
}
