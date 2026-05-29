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
        get_local_public_rooms(limit, since, filter, network).await
    }
}

async fn get_local_public_rooms(
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

    let search_term = filter
        .generic_search_term
        .as_ref()
        .map(|q| q.to_lowercase());
    let public_room_ids = room::public_room_ids().await?;
    let mut all_rooms: Vec<PublicRoomsChunk> = Vec::new();
    for room_id in public_room_ids.into_iter() {
        let build_chunk = async {
            let chunk = PublicRoomsChunk {
                canonical_alias: room::get_canonical_alias(&room_id).await.ok().flatten(),
                name: room::get_name(&room_id).await.ok(),
                num_joined_members: room::joined_member_count(&room_id).await.unwrap_or_else(|_| {
                    warn!("Room {} has no member count", room_id);
                    0
                }),
                topic: room::get_topic(&room_id).await.ok(),
                world_readable: room::is_world_readable(&room_id).await,
                guest_can_join: room::guest_can_join(&room_id).await,
                avatar_url: room::get_avatar_url(&room_id).await.ok().flatten(),
                join_rule: room::get_state_content::<RoomJoinRulesEventContent>(
                    &room_id,
                    &StateEventType::RoomJoinRules,
                    "",
                    None,
                )
                .await.map(|c| match c.join_rule {
                    JoinRule::Public => Some(PublicRoomJoinRule::Public),
                    JoinRule::Knock => Some(PublicRoomJoinRule::Knock),
                    JoinRule::KnockRestricted(..) => Some(PublicRoomJoinRule::KnockRestricted),
                    _ => None,
                })?
                .ok_or_else(|| AppError::public("Missing room join rule event for room."))?,
                room_type: room::get_room_type(&room_id).await.ok().flatten(),
                room_id,
            };
            Ok::<_, AppError>(chunk)
        };

        // Filter out buggy rooms
        let Ok(chunk) = build_chunk.await else {
            continue;
        };

        let matches = if let Some(search_term) = &search_term {
            if let Some(name) = &chunk.name
                && name.as_str().to_lowercase().contains(search_term)
            {
                true
            } else if let Some(topic) = &chunk.topic
                && topic.to_lowercase().contains(search_term)
            {
                true
            } else {
                chunk.canonical_alias.as_ref().is_some_and(|canonical_alias| {
                    canonical_alias
                        .as_str()
                        .to_lowercase()
                        .contains(search_term)
                })
            }
        } else {
            // No search term
            true
        };

        if matches {
            // We need to collect all, so we can sort by member count
            all_rooms.push(chunk);
        }
    }

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
