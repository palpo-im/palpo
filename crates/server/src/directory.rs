use crate::core::ServerName;
use crate::core::directory::{
    PublicRoomFilter, PublicRoomJoinRule, PublicRoomsChunk, PublicRoomsResBody, RoomNetwork,
};
use crate::core::events::StateEventType;
use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
use crate::core::federation::directory::{PublicRoomsReqBody, public_rooms_request};
use crate::core::room::JoinRule;
use crate::exts::*;
use crate::{AppError, AppResult, MatrixError, config, room};

pub async fn get_public_rooms(
    server: Option<&ServerName>,
    limit: Option<usize>,
    since: Option<&str>,
    filter: &PublicRoomFilter,
    network: &RoomNetwork,
) -> AppResult<PublicRoomsResBody> {
    if let Some(other_server) =
        server.filter(|server| *server != config::get().server_name.as_str())
    {
        let body = public_rooms_request(
            &other_server.origin().await,
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

    let mut all_rooms: Vec<_> = room::public_room_ids()?
        .into_iter()
        .map(|room_id| {
            let chunk = PublicRoomsChunk {
                canonical_alias: room::get_canonical_alias(&room_id).ok().flatten(),
                name: room::get_name(&room_id).ok(),
                num_joined_members: room::joined_member_count(&room_id).unwrap_or_else(|_| {
                    warn!("Room {} has no member count", room_id);
                    0
                }),
                topic: room::get_topic(&room_id).ok(),
                world_readable: room::is_world_readable(&room_id),
                guest_can_join: room::guest_can_join(&room_id),
                avatar_url: room::get_avatar_url(&room_id).ok().flatten(),
                join_rule: room::get_state_content::<RoomJoinRulesEventContent>(
                    &room_id,
                    &StateEventType::RoomJoinRules,
                    "",
                    None,
                )
                .map(|c| match c.join_rule {
                    JoinRule::Public => Some(PublicRoomJoinRule::Public),
                    JoinRule::Knock => Some(PublicRoomJoinRule::Knock),
                    JoinRule::KnockRestricted(..) => Some(PublicRoomJoinRule::KnockRestricted),
                    _ => None,
                })?
                .ok_or_else(|| AppError::public("Missing room join rule event for room."))?,
                room_type: room::get_room_type(&room_id).ok().flatten(),
                room_id,
            };
            Ok(chunk)
        })
        .filter_map(|r: AppResult<_>| r.ok()) // Filter out buggy rooms
        .filter(|chunk| {
            if let Some(query) = filter
                .generic_search_term
                .as_ref()
                .map(|q| q.to_lowercase())
            {
                if let Some(name) = &chunk.name
                    && name.as_str().to_lowercase().contains(&query)
                {
                    return true;
                }

                if let Some(topic) = &chunk.topic
                    && topic.to_lowercase().contains(&query)
                {
                    return true;
                }

                if let Some(canonical_alias) = &chunk.canonical_alias
                    && canonical_alias.as_str().to_lowercase().contains(&query)
                {
                    return true;
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
        .take(limit)
        .collect();

    let prev_batch = if num_since == 0 {
        None
    } else {
        Some(format!("p{num_since}"))
    };

    let next_batch = if chunk.len() < limit {
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
