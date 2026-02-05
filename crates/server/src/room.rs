use std::collections::HashSet;
use std::sync::OnceLock;

use diesel::prelude::*;
use serde::de::DeserializeOwned;

use crate::appservice::RegistrationInfo;
use crate::core::directory::RoomTypeFilter;
use crate::core::events::StateEventType;
use crate::core::events::room::avatar::RoomAvatarEventContent;
use crate::core::events::room::canonical_alias::RoomCanonicalAliasEventContent;
use crate::core::events::room::create::RoomCreateEventContent;
use crate::core::events::room::encryption::RoomEncryptionEventContent;
use crate::core::events::room::guest_access::{GuestAccess, RoomGuestAccessEventContent};
use crate::core::events::room::history_visibility::{
    HistoryVisibility, RoomHistoryVisibilityEventContent,
};
use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::room::name::RoomNameEventContent;
use crate::core::events::room::power_levels::{RoomPowerLevels, RoomPowerLevelsEventContent};
use crate::core::events::room::topic::RoomTopicEventContent;
use crate::core::identifiers::*;
use crate::core::room::{JoinRule, RoomType};
use crate::core::room_version_rules::RoomVersionRules;
use crate::core::state::events::RoomCreateEvent;
use crate::core::{Seqnum, UnixMillis};
use crate::data::room::{DbRoomCurrent, NewDbRoom};
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::{
    APPSERVICE_IN_ROOM_CACHE, AppError, AppResult, IsRemoteOrLocal, RoomMutexGuard, RoomMutexMap,
    SnPduEvent, config, data, membership, room, utils,
};

pub mod alias;
pub use alias::*;
pub mod auth_chain;
mod current;
pub mod directory;
pub mod lazy_loading;
pub mod pdu_metadata;
pub mod receipt;
pub mod space;
pub mod state;
pub mod timeline;
pub mod typing;
pub mod user;
pub use current::*;
pub mod push_action;
pub mod thread;
pub use state::get_room_frame_id as get_frame_id;

pub async fn lock_state(room_id: &RoomId) -> RoomMutexGuard {
    static ROOM_STATE_MUTEX: OnceLock<RoomMutexMap> = OnceLock::new();
    ROOM_STATE_MUTEX
        .get_or_init(Default::default)
        .lock(room_id)
        .await
}

pub fn create_room(new_room: NewDbRoom) -> AppResult<OwnedRoomId> {
    diesel::insert_into(rooms::table)
        .values(&new_room)
        .execute(&mut connect()?)?;
    Ok(new_room.id)
}

pub fn ensure_room(id: &RoomId, room_version_id: &RoomVersionId) -> AppResult<OwnedRoomId> {
    if room_exists(id)? {
        Ok(id.to_owned())
    } else {
        create_room(NewDbRoom {
            id: id.to_owned(),
            version: room_version_id.to_string(),
            is_public: false,
            min_depth: 0,
            has_auth_chain_index: false,
            created_at: UnixMillis::now(),
        })
    }
}

/// Checks if a room exists.
pub fn room_exists(room_id: &RoomId) -> AppResult<bool> {
    diesel_exists!(rooms::table.filter(rooms::id.eq(room_id)), &mut connect()?).map_err(Into::into)
}

pub fn get_room_sn(room_id: &RoomId) -> AppResult<Seqnum> {
    let room_sn = rooms::table
        .filter(rooms::id.eq(room_id))
        .select(rooms::sn)
        .first::<Seqnum>(&mut connect()?)?;
    Ok(room_sn)
}

/// Returns the room's version.
pub fn get_version(room_id: &RoomId) -> AppResult<RoomVersionId> {
    if let Some(room_version) = rooms::table
        .find(room_id)
        .select(rooms::version)
        .first::<String>(&mut connect()?)
        .optional()?
    {
        return Ok(RoomVersionId::try_from(&*room_version)?);
    }
    let create_event_content = get_state_content::<RoomCreateEventContent>(
        room_id,
        &StateEventType::RoomCreate,
        "",
        None,
    )?;
    Ok(create_event_content.room_version)
}

pub fn get_current_frame_id(room_id: &RoomId) -> AppResult<Option<i64>> {
    rooms::table
        .find(room_id)
        .select(rooms::state_frame_id)
        .first(&mut connect()?)
        .optional()
        .map(|v| v.flatten())
        .map_err(Into::into)
}

pub fn is_disabled(room_id: &RoomId) -> AppResult<bool> {
    rooms::table
        .filter(rooms::id.eq(room_id))
        .select(rooms::disabled)
        .first(&mut connect()?)
        .map_err(Into::into)
}

pub fn disable_room(room_id: &RoomId, disabled: bool) -> AppResult<()> {
    diesel::update(rooms::table.filter(rooms::id.eq(room_id)))
        .set(rooms::disabled.eq(disabled))
        .execute(&mut connect()?)
        .map(|_| ())
        .map_err(Into::into)
}

pub fn update_currents(room_id: &RoomId) -> AppResult<()> {
    let joined_members = room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq("join"))
        .count()
        .get_result::<i64>(&mut connect()?)?;
    let invited_members = room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq("invite"))
        .count()
        .get_result::<i64>(&mut connect()?)?;
    let left_members = room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq("leave"))
        .count()
        .get_result::<i64>(&mut connect()?)?;
    let banned_members = room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq("banned"))
        .count()
        .get_result::<i64>(&mut connect()?)?;
    let knocked_members = room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq("knocked"))
        .count()
        .get_result::<i64>(&mut connect()?)?;

    let current = DbRoomCurrent {
        room_id: room_id.to_owned(),
        state_events: 0, // TODO: fixme
        joined_members,
        invited_members,
        left_members,
        banned_members,
        knocked_members,
        local_users_in_room: 0,       // TODO: fixme
        completed_delta_stream_id: 0, // TODO: fixme
    };
    diesel::insert_into(stats_room_currents::table)
        .values(&current)
        .on_conflict(stats_room_currents::room_id)
        .do_update()
        .set(&current)
        .execute(&mut connect()?)?;

    Ok(())
}

pub fn update_joined_servers(room_id: &RoomId) -> AppResult<()> {
    let joined_servers = room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq("join"))
        .select(room_users::user_id)
        .distinct()
        .load::<OwnedUserId>(&mut connect()?)?
        .into_iter()
        .map(|user_id| user_id.server_name().to_owned())
        .collect::<HashSet<OwnedServerName>>()
        .into_iter()
        .collect::<Vec<_>>();

    diesel::delete(
        room_joined_servers::table
            .filter(room_joined_servers::room_id.eq(room_id))
            .filter(room_joined_servers::server_id.ne_all(&joined_servers)),
    )
    .execute(&mut connect()?)?;

    for joined_server in joined_servers {
        data::room::add_joined_server(room_id, &joined_server)?;
    }
    Ok(())
}
pub fn get_our_real_users(room_id: &RoomId) -> AppResult<Vec<OwnedUserId>> {
    room_users::table
        .filter(room_users::room_id.eq(room_id))
        .select(room_users::user_id)
        .load::<OwnedUserId>(&mut connect()?)
        .map_err(Into::into)
}

pub fn appservice_in_room(room_id: &RoomId, appservice: &RegistrationInfo) -> AppResult<bool> {
    let maybe = APPSERVICE_IN_ROOM_CACHE
        .read()
        .unwrap()
        .get(room_id)
        .and_then(|map| map.get(&appservice.registration.id))
        .copied();

    if let Some(b) = maybe {
        Ok(b)
    } else {
        let bridge_user_id = UserId::parse_with_server_name(
            appservice.registration.sender_localpart.as_str(),
            &config::get().server_name,
        )
        .ok();

        let bridge_user_joined = bridge_user_id
            .as_ref()
            .is_some_and(|id| user::is_joined(id, room_id).unwrap_or(false));

        let in_room = bridge_user_joined || {
            let user_ids = room_users::table
                .filter(room_users::room_id.eq(room_id))
                .select(room_users::user_id)
                .load::<String>(&mut connect()?)?;
            user_ids
                .iter()
                .any(|user_id| appservice.users.is_match(user_id.as_str()))
        };

        APPSERVICE_IN_ROOM_CACHE
            .write()
            .unwrap()
            .entry(room_id.to_owned())
            .or_default()
            .insert(appservice.registration.id.clone(), in_room);

        Ok(in_room)
    }
}
pub fn is_room_exists(room_id: &RoomId) -> AppResult<bool> {
    diesel_exists!(
        rooms::table.filter(rooms::id.eq(room_id)).select(rooms::id),
        &mut connect()?
    )
    .map_err(Into::into)
}
pub async fn should_join_on_remote_servers(
    sender_id: &UserId,
    room_id: &RoomId,
    servers: &[OwnedServerName],
) -> AppResult<(bool, Vec<OwnedServerName>)> {
    if room_id.is_local() {
        return Ok((false, vec![]));
    }
    if !is_server_joined(&config::get().server_name, room_id).unwrap_or(false) {
        return Ok((true, servers.to_vec()));
    }
    let Ok(join_rule) = room::get_join_rule(room_id) else {
        return Ok((true, servers.to_vec()));
    };

    if !join_rule.is_restricted() {
        return Ok((false, servers.to_vec()));
    }
    let users =
        membership::get_users_can_issue_invite(room_id, sender_id, &join_rule.restriction_rooms())
            .await?;
    let mut allowed_servers = crate::get_servers_from_users(&users);
    if let Ok(room_server) = room_id.server_name() {
        let room_server = room_server.to_owned();
        if !allowed_servers.contains(&room_server) {
            allowed_servers.push(room_server);
        }
    }
    Ok((true, allowed_servers))
}
pub fn is_server_joined(server: &ServerName, room_id: &RoomId) -> AppResult<bool> {
    let query = room_joined_servers::table
        .filter(room_joined_servers::room_id.eq(room_id))
        .filter(room_joined_servers::server_id.eq(server));
    diesel_exists!(query, &mut connect()?).map_err(Into::into)
}
pub fn joined_servers(room_id: &RoomId) -> AppResult<Vec<OwnedServerName>> {
    room_joined_servers::table
        .filter(room_joined_servers::room_id.eq(room_id))
        .select(room_joined_servers::server_id)
        .load::<OwnedServerName>(&mut connect()?)
        .map_err(Into::into)
}
pub fn has_any_other_server(room_id: &RoomId, server: &ServerName) -> AppResult<bool> {
    let query = room_joined_servers::table
        .filter(room_joined_servers::room_id.eq(room_id))
        .filter(room_joined_servers::server_id.ne(server));
    diesel_exists!(query, &mut connect()?).map_err(Into::into)
}

#[tracing::instrument(level = "trace")]
pub fn lookup_servers(room_id: &RoomId) -> AppResult<Vec<OwnedServerName>> {
    room_lookup_servers::table
        .filter(room_lookup_servers::room_id.eq(room_id))
        .select(room_lookup_servers::server_id)
        .load::<OwnedServerName>(&mut connect()?)
        .map_err(Into::into)
}

pub fn joined_member_count(room_id: &RoomId) -> AppResult<u64> {
    stats_room_currents::table
        .find(room_id)
        .select(stats_room_currents::joined_members)
        .first::<i64>(&mut connect()?)
        .optional()
        .map(|c| c.unwrap_or_default() as u64)
        .map_err(Into::into)
}

#[tracing::instrument]
pub fn invited_member_count(room_id: &RoomId) -> AppResult<u64> {
    stats_room_currents::table
        .find(room_id)
        .select(stats_room_currents::invited_members)
        .first::<i64>(&mut connect()?)
        .optional()
        .map(|c| c.unwrap_or_default() as u64)
        .map_err(Into::into)
}

pub fn joined_users(room_id: &RoomId, until_sn: Option<i64>) -> AppResult<Vec<OwnedUserId>> {
    get_state_users(room_id, &MembershipState::Join, until_sn)
}
pub fn invited_users(room_id: &RoomId, until_sn: Option<i64>) -> AppResult<Vec<OwnedUserId>> {
    get_state_users(room_id, &MembershipState::Invite, until_sn)
}
pub fn active_local_users_in_room(room_id: &RoomId) -> AppResult<Vec<OwnedUserId>> {
    // TODO: only active user?
    Ok(get_state_users(room_id, &MembershipState::Join, None)?
        .into_iter()
        .filter(|user_id| user_id.is_local())
        .collect())
}

pub fn list_banned_rooms() -> AppResult<Vec<OwnedRoomId>> {
    let room_ids = banned_rooms::table
        .select(banned_rooms::room_id)
        .load(&mut connect()?)?;
    Ok(room_ids)
}

pub fn get_state_users(
    room_id: &RoomId,
    state: &MembershipState,
    until_sn: Option<i64>,
) -> AppResult<Vec<OwnedUserId>> {
    if let Some(until_sn) = until_sn {
        room_users::table
            .filter(room_users::event_sn.le(until_sn))
            .filter(room_users::room_id.eq(room_id))
            .filter(room_users::membership.eq(state.to_string()))
            .select(room_users::user_id)
            .load(&mut connect()?)
            .map_err(Into::into)
    } else {
        room_users::table
            .filter(room_users::room_id.eq(room_id))
            .filter(room_users::membership.eq(state.to_string()))
            .select(room_users::user_id)
            .load(&mut connect()?)
            .map_err(Into::into)
    }
}
pub fn server_name(room_id: &RoomId) -> AppResult<OwnedServerName> {
    if let Ok(server_name) = room_id.server_name() {
        return Ok(server_name.to_owned());
    }
    let create_event = get_create(room_id)?;
    Ok(create_event.creator()?.server_name().to_owned())
}

/// Returns an list of all servers participating in this room.
pub fn participating_servers(
    room_id: &RoomId,
    include_self_server: bool,
) -> AppResult<Vec<OwnedServerName>> {
    if include_self_server {
        room_joined_servers::table
            .filter(room_joined_servers::room_id.eq(room_id))
            .select(room_joined_servers::server_id)
            .load(&mut connect()?)
            .map_err(Into::into)
    } else {
        room_joined_servers::table
            .filter(room_joined_servers::room_id.eq(room_id))
            .filter(room_joined_servers::server_id.ne(config::server_name()))
            .select(room_joined_servers::server_id)
            .load(&mut connect()?)
            .map_err(Into::into)
    }
}

pub fn admin_servers(
    room_id: &RoomId,
    include_self_server: bool,
) -> AppResult<Vec<OwnedServerName>> {
    let power_levels = get_state_content::<RoomPowerLevelsEventContent>(
        room_id,
        &StateEventType::RoomPowerLevels,
        "",
        None,
    )?;
    let mut admin_servers = power_levels
        .users
        .iter()
        .filter(|(_, level)| **level > power_levels.users_default)
        .map(|(user_id, _)| user_id.server_name())
        .collect::<HashSet<_>>();
    if !include_self_server {
        admin_servers.remove(config::server_name());
    }
    Ok(admin_servers.into_iter().map(|s| s.to_owned()).collect())
}

pub fn public_room_ids() -> AppResult<Vec<OwnedRoomId>> {
    rooms::table
        .filter(rooms::is_public.eq(true))
        .select(rooms::id)
        .order_by(rooms::sn.desc())
        .load(&mut connect()?)
        .map_err(Into::into)
}
pub fn all_room_ids() -> AppResult<Vec<OwnedRoomId>> {
    rooms::table
        .select(rooms::id)
        .load(&mut connect()?)
        .map_err(Into::into)
}

pub fn filter_rooms<'a>(
    rooms: &[&'a RoomId],
    filter: &[RoomTypeFilter],
    negate: bool,
) -> Vec<&'a RoomId> {
    rooms
        .iter()
        .filter_map(|r| {
            let r = *r;
            let room_type = get_room_type(r).ok()?;
            let room_type_filter = RoomTypeFilter::from(room_type);

            let include = if negate {
                !filter.contains(&room_type_filter)
            } else {
                filter.is_empty() || filter.contains(&room_type_filter)
            };

            include.then_some(r)
        })
        .collect()
}

pub async fn room_available_servers(
    room_id: &RoomId,
    room_alias: &RoomAliasId,
    pre_servers: Vec<OwnedServerName>,
) -> AppResult<Vec<OwnedServerName>> {
    // find active servers in room state cache to suggest
    let mut servers: Vec<OwnedServerName> = joined_servers(room_id)?;

    // push any servers we want in the list already (e.g. responded remote alias
    // servers, room alias server itself)
    servers.extend(pre_servers);

    servers.sort_unstable();
    servers.dedup();

    // shuffle list of servers randomly after sort and dedup
    utils::shuffle(&mut servers);

    // insert our server as the very first choice if in list, else check if we can
    // prefer the room alias server first
    match servers
        .iter()
        .position(|server_name| server_name.is_local())
    {
        Some(server_index) => {
            servers.swap_remove(server_index);
            servers.insert(0, config::get().server_name.to_owned());
        }
        _ => {
            if let Some(alias_server_index) = servers
                .iter()
                .position(|server| server == room_alias.server_name())
            {
                servers.swap_remove(alias_server_index);
                servers.insert(0, room_alias.server_name().into());
            }
        }
    }

    Ok(servers)
}

pub fn get_state(
    room_id: &RoomId,
    event_type: &StateEventType,
    state_key: &str,
    until_sn: Option<Seqnum>,
) -> AppResult<SnPduEvent> {
    let frame_id = get_frame_id(room_id, until_sn)?;
    state::get_state(frame_id, event_type, state_key)
}

pub fn get_state_content<T>(
    room_id: &RoomId,
    event_type: &StateEventType,
    state_key: &str,
    until_sn: Option<Seqnum>,
) -> AppResult<T>
where
    T: DeserializeOwned,
{
    let frame_id = get_frame_id(room_id, until_sn)?;
    state::get_state_content(frame_id, event_type, state_key)
}

pub fn get_create(room_id: &RoomId) -> AppResult<RoomCreateEvent<SnPduEvent>> {
    get_state(room_id, &StateEventType::RoomCreate, "", None).map(RoomCreateEvent::new)
}

pub fn get_name(room_id: &RoomId) -> AppResult<String> {
    get_state_content::<RoomNameEventContent>(room_id, &StateEventType::RoomName, "", None)
        .map(|c| c.name)
}

pub fn get_avatar_url(room_id: &RoomId) -> AppResult<Option<OwnedMxcUri>> {
    get_state_content::<RoomAvatarEventContent>(room_id, &StateEventType::RoomAvatar, "", None)
        .map(|c| c.url)
}

pub fn get_member(
    room_id: &RoomId,
    user_id: &UserId,
    until_sn: Option<Seqnum>,
) -> AppResult<RoomMemberEventContent> {
    get_state_content::<RoomMemberEventContent>(
        room_id,
        &StateEventType::RoomMember,
        user_id.as_str(),
        until_sn,
    )
}
pub fn get_topic(room_id: &RoomId) -> AppResult<String> {
    get_topic_content(room_id).map(|c| c.topic)
}
pub fn get_topic_content(room_id: &RoomId) -> AppResult<RoomTopicEventContent> {
    get_state_content::<RoomTopicEventContent>(room_id, &StateEventType::RoomTopic, "", None)
}
pub fn get_canonical_alias(room_id: &RoomId) -> AppResult<Option<OwnedRoomAliasId>> {
    get_state_content::<RoomCanonicalAliasEventContent>(
        room_id,
        &StateEventType::RoomCanonicalAlias,
        "",
        None,
    )
    .map(|c| c.alias)
}
pub fn get_join_rule(room_id: &RoomId) -> AppResult<JoinRule> {
    get_state_content::<RoomJoinRulesEventContent>(
        room_id,
        &StateEventType::RoomJoinRules,
        "",
        None,
    )
    .map(|c| c.join_rule)
}
pub async fn get_power_levels(room_id: &RoomId) -> AppResult<RoomPowerLevels> {
    let create = get_create(room_id)?;
    let room_version = create.room_version()?;
    let version_rules = crate::room::get_version_rules(&room_version)?;
    let creators = create.creators()?;

    let content = get_power_levels_event_content(room_id)?;
    let power_levels = RoomPowerLevels::new(content.into(), &version_rules.authorization, creators);
    Ok(power_levels)
}
pub fn get_power_levels_event_content(room_id: &RoomId) -> AppResult<RoomPowerLevelsEventContent> {
    get_state_content::<RoomPowerLevelsEventContent>(
        room_id,
        &StateEventType::RoomPowerLevels,
        "",
        None,
    )
}

pub fn get_room_type(room_id: &RoomId) -> AppResult<Option<RoomType>> {
    get_state_content::<RoomCreateEventContent>(room_id, &StateEventType::RoomCreate, "", None)
        .map(|c| c.room_type)
}

pub fn get_history_visibility(room_id: &RoomId) -> AppResult<HistoryVisibility> {
    get_state_content::<RoomHistoryVisibilityEventContent>(
        room_id,
        &StateEventType::RoomHistoryVisibility,
        "",
        None,
    )
    .map(|c| c.history_visibility)
}

pub fn is_world_readable(room_id: &RoomId) -> bool {
    get_history_visibility(room_id)
        .map(|visibility| visibility == HistoryVisibility::WorldReadable)
        .unwrap_or(false)
}
pub fn guest_can_join(room_id: &RoomId) -> bool {
    get_state_content::<RoomGuestAccessEventContent>(
        room_id,
        &StateEventType::RoomGuestAccess,
        "",
        None,
    )
    .map(|c| c.guest_access == GuestAccess::CanJoin)
    .unwrap_or(false)
}

pub async fn user_can_invite(room_id: &RoomId, sender_id: &UserId, _target_user: &UserId) -> bool {
    if let Ok(create_event) = crate::room::get_create(room_id)
        && let Ok(creators) = create_event.creators()
        && creators.contains(sender_id)
    {
        return true;
    }
    if let Ok(power_levels) = get_power_levels(room_id).await {
        power_levels.user_can_invite(sender_id)
    } else {
        false
    }
}

pub fn get_encryption(room_id: &RoomId) -> AppResult<EventEncryptionAlgorithm> {
    get_state_content(room_id, &StateEventType::RoomEncryption, "", None)
        .map(|content: RoomEncryptionEventContent| content.algorithm)
}

pub fn is_encrypted(room_id: &RoomId) -> bool {
    get_state(room_id, &StateEventType::RoomEncryption, "", None).is_ok()
}

/// Gets the room ID of the admin room
///
/// Errors are propagated from the database, and will have None if there is no admin room
pub fn get_admin_room() -> AppResult<OwnedRoomId> {
    crate::room::resolve_local_alias(config::admin_alias())
}

pub fn is_admin_room(room_id: &RoomId) -> AppResult<bool> {
    let result = get_admin_room();
    match result {
        Ok(admin_room_id) => Ok(admin_room_id == room_id),
        Err(e) => {
            if e.is_not_found() {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

/// Returns an iterator of all our local users in the room, even if they're
/// deactivated/guests
#[tracing::instrument(level = "debug")]
// TODO: local?
pub fn local_users_in_room(room_id: &RoomId) -> AppResult<Vec<OwnedUserId>> {
    room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::user_server_id.eq(&config::get().server_name))
        .select(room_users::user_id)
        .load::<OwnedUserId>(&mut connect()?)
        .map_err(Into::into)
}

/// Returns an iterator of all our local users in the room, even if they're
/// deactivated/guests
#[tracing::instrument(level = "debug")]
pub fn get_members(room_id: &RoomId) -> AppResult<Vec<OwnedUserId>> {
    room_users::table
        .filter(room_users::room_id.eq(room_id))
        .select(room_users::user_id)
        .load::<OwnedUserId>(&mut connect()?)
        .map_err(Into::into)
}

pub fn keys_changed_users(
    room_id: &RoomId,
    since_sn: i64,
    until_sn: Option<i64>,
) -> AppResult<Vec<OwnedUserId>> {
    if let Some(until_sn) = until_sn {
        e2e_key_changes::table
            .filter(e2e_key_changes::room_id.eq(room_id))
            .filter(e2e_key_changes::occur_sn.ge(since_sn))
            .filter(e2e_key_changes::occur_sn.le(until_sn))
            .select(e2e_key_changes::user_id)
            .load::<OwnedUserId>(&mut connect()?)
            .map_err(Into::into)
    } else {
        e2e_key_changes::table
            .filter(e2e_key_changes::room_id.eq(room_id.as_str()))
            .filter(e2e_key_changes::occur_sn.ge(since_sn))
            .select(e2e_key_changes::user_id)
            .load::<OwnedUserId>(&mut connect()?)
            .map_err(Into::into)
    }
}

pub fn ban_room(room_id: &RoomId, banned: bool) -> AppResult<()> {
    if banned {
        diesel::insert_into(banned_rooms::table)
            .values((
                banned_rooms::room_id.eq(room_id),
                banned_rooms::created_at.eq(UnixMillis::now()),
            ))
            .on_conflict_do_nothing()
            .execute(&mut connect()?)
            .map(|_| ())
            .map_err(Into::into)
    } else {
        diesel::delete(banned_rooms::table.filter(banned_rooms::room_id.eq(room_id)))
            .execute(&mut connect()?)
            .map(|_| ())
            .map_err(Into::into)
    }
}

pub fn get_version_rules(room_version: &RoomVersionId) -> AppResult<RoomVersionRules> {
    room_version.rules().ok_or_else(|| {
        AppError::public(format!(
            "Cannot verify event for unknown room version {room_version:?}."
        ))
    })
}
