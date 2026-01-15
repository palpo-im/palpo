use std::collections::{BTreeSet, VecDeque};
use std::str::FromStr;
use std::sync::{LazyLock, Mutex, MutexGuard};

use lru_cache::LruCache;

use crate::core::client::space::SpaceHierarchyRoomsChunk;
use crate::core::events::StateEventType;
use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
use crate::core::events::space::child::{HierarchySpaceChildEvent, SpaceChildEventContent};
use crate::core::federation::space::{
    HierarchyReqArgs, HierarchyResBody, SpaceHierarchyChildSummary, SpaceHierarchyParentSummary,
    hierarchy_request,
};
use crate::core::identifiers::*;
use crate::core::room::{JoinRule, RoomType};
use crate::core::serde::RawJson;
use crate::core::space::SpaceRoomJoinRule;
use crate::core::{self, OwnedRoomId, RoomId, UserId};
use crate::event::handler;
use crate::room::state;
use crate::{AppResult, GetUrlOrigin, MatrixError};

mod pagination_token;
pub use pagination_token::PaginationToken;

use super::state::get_full_state;

type CacheItem = LruCache<(OwnedRoomId, bool), Option<CachedSpaceHierarchySummary>>;
pub static ROOM_ID_SPACE_CHUNK_CACHE: LazyLock<Mutex<CacheItem>> =
    LazyLock::new(|| Mutex::new(LruCache::new(100)));

pub struct CachedSpaceHierarchySummary {
    summary: SpaceHierarchyParentSummary,
}

#[derive(Clone, Debug)]
pub enum SummaryAccessibility {
    Accessible(SpaceHierarchyParentSummary),
    Inaccessible,
}

/// Identifier used to check if rooms are accessible. None is used if you want
/// to return the room, no matter if accessible or not
#[derive(Debug)]
pub enum Identifier<'a> {
    UserId(&'a UserId),
    ServerName(&'a ServerName),
}

/// Gets the summary of a space using solely local information
pub async fn get_summary_and_children_local(
    current_room: &RoomId,
    identifier: &Identifier<'_>,
    suggested_only: bool,
) -> AppResult<Option<SummaryAccessibility>> {
    match ROOM_ID_SPACE_CHUNK_CACHE
        .lock()
        .unwrap()
        .get_mut(&(current_room.to_owned(), suggested_only))
        .as_ref()
    {
        None => (), // cache miss
        Some(None) => return Ok(None),
        Some(Some(cached)) => {
            let accessibility = if is_accessible_child(
                current_room,
                &cached.summary.join_rule,
                identifier,
                &cached.summary.allowed_room_ids,
            ) {
                SummaryAccessibility::Accessible(cached.summary.clone())
            } else {
                SummaryAccessibility::Inaccessible
            };
            return Ok(Some(accessibility));
        }
    }

    let children_pdus: Vec<_> = get_stripped_space_child_events(current_room, suggested_only)?;

    let Ok(summary) = get_room_summary(current_room, children_pdus, identifier).await else {
        return Ok(None);
    };

    ROOM_ID_SPACE_CHUNK_CACHE.lock().unwrap().insert(
        (current_room.to_owned(), suggested_only),
        Some(CachedSpaceHierarchySummary {
            summary: summary.clone(),
        }),
    );

    Ok(Some(SummaryAccessibility::Accessible(summary)))
}

/// Gets the summary of a space using solely federation
#[tracing::instrument(level = "debug")]
async fn get_summary_and_children_federation(
    current_room: &RoomId,
    suggested_only: bool,
    user_id: &UserId,
    via: &[OwnedServerName],
) -> AppResult<Option<SummaryAccessibility>> {
    let mut res_body = None;
    for server in via {
        let request = hierarchy_request(
            &server.origin().await,
            HierarchyReqArgs {
                room_id: current_room.to_owned(),
                suggested_only,
            },
        )?
        .into_inner();

        if let Ok(response) = crate::sending::send_federation_request(server, request, None).await
            && let Ok(body) = response.json::<HierarchyResBody>().await
        {
            ROOM_ID_SPACE_CHUNK_CACHE.lock().unwrap().insert(
                (current_room.to_owned(), suggested_only),
                Some(CachedSpaceHierarchySummary {
                    summary: body.room.clone(),
                }),
            );
            res_body = Some(body);
        }
        if res_body.is_some() {
            break;
        }
    }
    let Some(res_body) = res_body else {
        return Ok(None);
    };

    res_body
        .children
        .into_iter()
        .filter_map(|child| {
            if let Ok(mut cache) = ROOM_ID_SPACE_CHUNK_CACHE.lock() {
                if !cache.contains_key(&(current_room.to_owned(), suggested_only)) {
                    Some((child, cache))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .for_each(|(child, cache)| cache_insert(cache, current_room, child, suggested_only));

    let summary = res_body.room;
    let identifier = Identifier::UserId(user_id);
    let is_accessible_child = is_accessible_child(
        current_room,
        &summary.join_rule,
        &identifier,
        &summary.allowed_room_ids,
    );

    if is_accessible_child {
        return Ok(Some(SummaryAccessibility::Accessible(summary)));
    }

    Ok(Some(SummaryAccessibility::Inaccessible))
}

/// Simply returns the stripped m.space.child events of a room
fn get_stripped_space_child_events(
    room_id: &RoomId,
    suggested_only: bool,
) -> AppResult<Vec<RawJson<HierarchySpaceChildEvent>>> {
    let frame_id = super::get_frame_id(room_id, None)?;
    let child_events = get_full_state(frame_id)?
        .into_iter()
        .filter_map(|((state_event_type, state_key), pdu)| {
            if state_event_type == StateEventType::SpaceChild {
                let content = pdu.get_content::<SpaceChildEventContent>().ok()?;

                if content.via.is_empty() || (suggested_only && !content.suggested) {
                    return None;
                }

                if RoomId::parse(&state_key).is_ok() {
                    return Some(pdu.to_stripped_space_child_event());
                }
            }

            None
        })
        .collect();
    Ok(child_events)
}

/// Gets the summary of a space using either local or remote (federation)
/// sources
pub async fn get_summary_and_children_client(
    current_room: &OwnedRoomId,
    suggested_only: bool,
    user_id: &UserId,
    via: &[OwnedServerName],
) -> AppResult<Option<SummaryAccessibility>> {
    let identifier = Identifier::UserId(user_id);

    if let Ok(Some(response)) =
        get_summary_and_children_local(current_room, &identifier, suggested_only).await
    {
        return Ok(Some(response));
    }

    get_summary_and_children_federation(current_room, suggested_only, user_id, via).await
}

async fn get_room_summary(
    room_id: &RoomId,
    children_state: Vec<RawJson<HierarchySpaceChildEvent>>,
    identifier: &Identifier<'_>,
) -> AppResult<SpaceHierarchyParentSummary> {
    let join_rule = super::get_state_content::<RoomJoinRulesEventContent>(
        room_id,
        &StateEventType::RoomJoinRules,
        "",
        None,
    )
    .map_or(JoinRule::Invite, |c: RoomJoinRulesEventContent| c.join_rule);

    let allowed_room_ids = state::allowed_room_ids(join_rule.clone());

    let join_rule: SpaceRoomJoinRule = join_rule.clone().into();
    let is_accessible_child =
        is_accessible_child(room_id, &join_rule, identifier, &allowed_room_ids);

    if !is_accessible_child {
        return Err(MatrixError::forbidden("User is not allowed to see the room", None).into());
    }

    let name = super::get_name(room_id).ok();
    let topic = super::get_topic(room_id).ok();
    let room_type = super::get_room_type(room_id).ok().flatten();
    let world_readable = super::is_world_readable(room_id);
    let guest_can_join = super::guest_can_join(room_id);
    let num_joined_members = super::joined_member_count(room_id).unwrap_or(0);
    let canonical_alias = super::get_canonical_alias(room_id).ok().flatten();
    let avatar_url = super::get_avatar_url(room_id).ok().flatten();
    let room_version = super::get_version(room_id).ok();
    let encryption = super::get_encryption(room_id).ok();

    Ok(SpaceHierarchyParentSummary {
        canonical_alias,
        name,
        topic,
        world_readable,
        guest_can_join,
        avatar_url,
        room_type,
        children_state,
        allowed_room_ids,
        join_rule,
        room_id: room_id.to_owned(),
        num_joined_members,
        room_version,
        encryption,
    })
}

pub async fn get_room_hierarchy(
    sender_id: &UserId,
    args: &core::client::space::HierarchyReqArgs,
) -> AppResult<core::client::space::HierarchyResBody> {
    type Entry = (OwnedRoomId, Vec<OwnedServerName>);
    type RoomDeque = VecDeque<Entry>;

    let limit = args.limit.unwrap_or(50).min(50);
    let max_depth = args.max_depth.unwrap_or(usize::MAX);
    let pagination_token = args
        .from
        .as_ref()
        .and_then(|s| PaginationToken::from_str(s).ok());

    // Should prevent unexpeded behaviour in (bad) clients
    if let Some(token) = &pagination_token
        && (token.suggested_only != args.suggested_only || token.max_depth != max_depth)
    {
        return Err(MatrixError::invalid_param(
            "suggested_only and max_depth cannot change on paginated requests",
        )
        .into());
    }

    let room_sns = pagination_token.map(|p| p.room_sns).unwrap_or_default();
    let room_id = &args.room_id;
    let suggested_only = args.suggested_only;
    let mut queue: RoomDeque =
        [(room_id.to_owned(), vec![crate::room::server_name(room_id)?])].into();

    let mut rooms = Vec::with_capacity(limit);
    let mut parents = BTreeSet::new();
    while let Some((current_room, via)) = queue.pop_front() {
        let summary = match crate::room::space::get_summary_and_children_client(
            &current_room,
            suggested_only,
            sender_id,
            &via,
        )
        .await
        {
            Ok(summary) => summary,
            Err(e) => {
                error!("failed to get space summary for {}: {}", current_room, e);
                None
            }
        };

        match (summary, &current_room == room_id) {
            (None | Some(SummaryAccessibility::Inaccessible), false) => {
                // Just ignore other unavailable rooms
            }
            (None, true) => {
                return Err(
                    MatrixError::forbidden("the requested room was not found", None).into(),
                );
            }
            (Some(SummaryAccessibility::Inaccessible), true) => {
                return Err(
                    MatrixError::forbidden("the requested room is inaccessible", None).into(),
                );
            }
            (Some(SummaryAccessibility::Accessible(summary)), _) => {
                let populate = parents.len() >= room_sns.len();

                let mut children = Vec::new();
                let room_type = crate::room::get_room_type(&current_room).ok();
                if room_type.is_none() || Some(Some(RoomType::Space)) == room_type {
                    children = get_parent_children_via(&summary, suggested_only)
                        .into_iter()
                        .filter(|(room, _)| !parents.contains(room))
                        .collect::<Vec<Entry>>();
                }

                if populate {
                    rooms.push(summary_to_chunk(summary.clone()));
                } else if queue.is_empty() && children.is_empty() {
                    break;
                }

                if rooms.len() >= limit {
                    break;
                }

                parents.insert(current_room.clone());

                if parents.len() > max_depth {
                    continue;
                }

                for child in children.into_iter().rev() {
                    queue.push_front(child);
                }
            }
        }
    }

    let next_batch = if let Some((room, _)) = queue.pop_front() {
        parents.insert(room);

        let next_room_sns: Vec<_> = parents
            .iter()
            .filter_map(|room_id| crate::room::get_room_sn(room_id).ok())
            .collect();

        if !next_room_sns.is_empty() && next_room_sns.iter().ne(&room_sns) {
            Some(
                PaginationToken {
                    room_sns: next_room_sns,
                    limit,
                    max_depth,
                    suggested_only,
                }
                .to_string(),
            )
        } else {
            None
        }
    } else {
        None
    };
    Ok(core::client::space::HierarchyResBody { next_batch, rooms })
}

/// With the given identifier, checks if a room is accessable
fn is_accessible_child(
    current_room: &RoomId,
    join_rule: &SpaceRoomJoinRule,
    identifier: &Identifier<'_>,
    allowed_room_ids: &[OwnedRoomId],
) -> bool {
    // Checks if ACLs allow for the server to participate
    if let Identifier::ServerName(server_name) = identifier
        && handler::acl_check(server_name, current_room).is_err()
    {
        return false;
    }

    if let Identifier::UserId(user_id) = identifier
        && (crate::room::user::is_joined(user_id, current_room).unwrap_or(false)
            || crate::room::user::is_invited(user_id, current_room).unwrap_or(false))
    {
        return true;
    }

    match join_rule {
        SpaceRoomJoinRule::Public
        | SpaceRoomJoinRule::Knock
        | SpaceRoomJoinRule::KnockRestricted => true,
        SpaceRoomJoinRule::Restricted => allowed_room_ids.iter().any(|room| match identifier {
            Identifier::UserId(user) => crate::room::user::is_joined(user, room).unwrap_or(false),
            Identifier::ServerName(server) => {
                crate::room::is_server_joined(server, room).unwrap_or(false)
            }
        }),

        // Invite only, Private, or Custom join rule
        _ => false,
    }
}

/// Returns the children of a SpaceHierarchyParentSummary, making use of the
/// children_state field
pub fn get_parent_children_via(
    parent: &SpaceHierarchyParentSummary,
    suggested_only: bool,
) -> Vec<(OwnedRoomId, Vec<OwnedServerName>)> {
    parent
        .children_state
        .iter()
        .filter_map(|state| {
            if let Ok(ce) = RawJson::deserialize(state) {
                (!suggested_only || ce.content.suggested).then_some((ce.state_key, ce.content.via))
            } else {
                None
            }
        })
        .collect()
}

fn cache_insert(
    mut cache: MutexGuard<'_, CacheItem>,
    current_room: &RoomId,
    child: SpaceHierarchyChildSummary,
    suggested_only: bool,
) {
    let SpaceHierarchyChildSummary {
        canonical_alias,
        name,
        num_joined_members,
        room_id,
        topic,
        world_readable,
        guest_can_join,
        avatar_url,
        join_rule,
        room_type,
        allowed_room_ids,
        room_version,
        encryption,
    } = child;

    let summary = SpaceHierarchyParentSummary {
        canonical_alias,
        name,
        num_joined_members,
        topic,
        world_readable,
        guest_can_join,
        avatar_url,
        join_rule,
        room_type,
        allowed_room_ids,
        room_id: room_id.clone(),
        children_state: get_stripped_space_child_events(&room_id, suggested_only)
            .unwrap_or_default(),
        room_version,
        encryption,
    };

    cache.insert(
        (current_room.to_owned(), suggested_only),
        Some(CachedSpaceHierarchySummary { summary }),
    );
}

// Here because cannot implement `From` across palpo-federation-api and
// palpo-client-api types
impl From<CachedSpaceHierarchySummary> for SpaceHierarchyRoomsChunk {
    fn from(value: CachedSpaceHierarchySummary) -> Self {
        let SpaceHierarchyParentSummary {
            canonical_alias,
            name,
            num_joined_members,
            room_id,
            topic,
            world_readable,
            guest_can_join,
            avatar_url,
            join_rule,
            room_type,
            children_state,
            encryption,
            room_version,
            allowed_room_ids,
        } = value.summary;

        Self {
            canonical_alias,
            name,
            num_joined_members,
            room_id,
            topic,
            world_readable,
            guest_can_join,
            avatar_url,
            join_rule,
            room_type,
            children_state,
            encryption,
            room_version,
            allowed_room_ids,
        }
    }
}

/// Here because cannot implement `From` across palpo-federation-api and
/// palpo-client-api types
#[must_use]
pub fn summary_to_chunk(summary: SpaceHierarchyParentSummary) -> SpaceHierarchyRoomsChunk {
    let SpaceHierarchyParentSummary {
        canonical_alias,
        name,
        num_joined_members,
        room_id,
        topic,
        world_readable,
        guest_can_join,
        avatar_url,
        join_rule,
        room_type,
        children_state,
        encryption,
        room_version,
        allowed_room_ids,
    } = summary;

    SpaceHierarchyRoomsChunk {
        canonical_alias,
        name,
        num_joined_members,
        room_id,
        topic,
        world_readable,
        guest_can_join,
        avatar_url,
        join_rule,
        room_type,
        children_state,
        encryption,
        room_version,
        allowed_room_ids,
    }
}
