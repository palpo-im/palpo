pub mod device;
pub use device::{DbUserDevice, NewDbUserDevice};
mod password;
pub use password::*;
mod profile;
pub use profile::*;
mod filter;
pub use filter::*;
mod access_token;
pub use access_token::*;
mod refresh_token;
pub use refresh_token::*;
mod data;
pub use data::*;
pub mod key;
pub mod pusher;
// pub mod push_rule;
// pub mod push_rule::*;
pub use key::*;
pub mod key_backup;
pub use key_backup::*;
pub mod session;
pub use session::*;
pub mod external_id;
pub mod known;
pub mod login_token;
pub mod openid_token;
pub mod presence;
pub mod registration_token;
pub mod uiaa;
use std::mem;

use diesel::dsl;
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
pub use external_id::*;
pub use presence::*;
pub use registration_token::*;

use crate::core::client::dehydrated_device::DehydratedDeviceData;
use crate::core::events::AnyStrippedStateEvent;
use crate::core::identifiers::*;
use crate::core::serde::{JsonValue, RawJson};
use crate::core::{OwnedMxcUri, UnixMillis};
use crate::schema::*;
use crate::{DataError, DataResult, connect};

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = users)]
pub struct DbUser {
    pub id: OwnedUserId,
    pub ty: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub is_local: bool,
    pub localpart: String,
    pub server_name: OwnedServerName,
    pub appservice_id: Option<String>,
    pub shadow_banned: bool,
    pub consent_at: Option<UnixMillis>,
    pub consent_version: Option<String>,
    pub consent_server_notice_sent: Option<String>,
    pub approved_at: Option<UnixMillis>,
    pub approved_by: Option<OwnedUserId>,
    pub deactivated_at: Option<UnixMillis>,
    pub deactivated_by: Option<OwnedUserId>,
    pub locked_at: Option<UnixMillis>,
    pub locked_by: Option<OwnedUserId>,
    pub created_at: UnixMillis,
    pub suspended_at: Option<UnixMillis>,
}

#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = users)]
pub struct NewDbUser {
    pub id: OwnedUserId,
    pub ty: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub is_local: bool,
    pub localpart: String,
    pub server_name: OwnedServerName,
    pub appservice_id: Option<String>,
    pub created_at: UnixMillis,
}

impl DbUser {
    pub fn is_deactivated(&self) -> bool {
        self.deactivated_at.is_some()
    }

    pub fn is_locked(&self) -> bool {
        self.locked_at.is_some()
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended_at.is_some()
    }
}

#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = user_ignores)]
pub struct NewDbUserIgnore {
    pub user_id: OwnedUserId,
    pub ignored_id: OwnedUserId,
    pub created_at: UnixMillis,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_threepids)]
pub struct NewDbUserThreepid {
    pub user_id: OwnedUserId,
    pub medium: String,
    pub address: String,
    pub validated_at: UnixMillis,
    pub added_at: UnixMillis,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_dehydrated_devices)]
pub struct DbUserDehydratedDevice {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub device_data: JsonValue,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_dehydrated_devices)]
pub struct NewDbUserDehydratedDevice {
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub device_data: JsonValue,
}

pub async fn is_admin(user_id: &UserId) -> DataResult<bool> {
    users::table
        .filter(users::id.eq(user_id))
        .select(users::is_admin)
        .first::<bool>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Returns an iterator over all rooms this user joined.
pub async fn joined_rooms(user_id: &UserId) -> DataResult<Vec<OwnedRoomId>> {
    let room_memeberships = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .distinct_on(room_users::room_id)
        .select((room_users::room_id, room_users::membership))
        .order_by((room_users::room_id.desc(), room_users::id.desc()))
        .load::<(OwnedRoomId, String)>(&mut connect().await?)
        .await?;
    Ok(room_memeberships
        .into_iter()
        .filter_map(|(room_id, membership)| {
            if membership == "join" {
                Some(room_id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>())
}

pub async fn ignored_users(user_id: &UserId) -> DataResult<Vec<OwnedUserId>> {
    user_ignores::table
        .filter(user_ignores::user_id.eq(user_id))
        .select(user_ignores::ignored_id)
        .load::<OwnedUserId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Whether the user's stable invite permission account data blocks all invites.
pub async fn invite_blocked(user_id: &UserId) -> DataResult<bool> {
    let config = get_global_data::<JsonValue>(user_id, "m.invite_permission_config").await?;
    Ok(config
        .as_ref()
        .and_then(|config| config.get("default_action"))
        .and_then(JsonValue::as_str)
        == Some("block"))
}

/// One invitation event: identity, sender, state and admission come from one SQL row.
pub struct SyncInvitation {
    pub membership_id: i64,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub room_id: OwnedRoomId,
    pub sender_id: OwnedUserId,
    pub state: Vec<RawJson<AnyStrippedStateEvent>>,
    pub admitted_sn: Option<i64>,
    pub delivered_sn: Option<i64>,
}

/// A shared-room membership snapshot pins the immutable room-state frame as well.
pub struct SharedInviteRoom {
    pub inviter: OwnedUserId,
    pub frame_id: i64,
}

pub struct InvitePermissionSnapshot {
    pub default_action: Option<String>,
    pub shared_rooms: Vec<SharedInviteRoom>,
}

async fn read_invite_permission(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: &UserId,
) -> DataResult<Option<DbUserData>> {
    Ok(user_datas::table
        .filter(user_datas::user_id.eq(user_id))
        .filter(user_datas::room_id.is_null())
        .filter(user_datas::data_type.eq("m.invite_permission_config"))
        .order_by(user_datas::id.desc())
        .first::<DbUserData>(conn)
        .await
        .optional()?)
}

fn invite_default_action(config: Option<&DbUserData>) -> Option<String> {
    config
        .filter(|config| !config.is_deleted)
        .and_then(|config| config.json_data.get("default_action"))
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

async fn read_invite_joined_rooms(
    conn: &mut diesel_async::AsyncPgConnection,
    invitee: &UserId,
    until_sn: i64,
) -> DataResult<Vec<OwnedRoomId>> {
    let recipient_rooms = room_users::table
        .filter(room_users::user_id.eq(invitee))
        .distinct_on(room_users::room_id)
        .order_by((room_users::room_id.desc(), room_users::id.desc()))
        .select((
            room_users::room_id,
            room_users::membership,
            room_users::event_sn,
        ))
        .load::<(OwnedRoomId, String, i64)>(conn)
        .await?;
    Ok(recipient_rooms
        .into_iter()
        .filter(|(_, membership, event_sn)| membership == "join" && *event_sn <= until_sn)
        .map(|(room_id, ..)| room_id)
        .collect())
}

async fn read_shared_invite_memberships(
    conn: &mut diesel_async::AsyncPgConnection,
    invitee: &UserId,
    inviters: &[OwnedUserId],
    until_sn: i64,
) -> DataResult<Vec<(OwnedUserId, OwnedRoomId, Option<i64>)>> {
    if inviters.is_empty() {
        return Ok(Vec::new());
    }
    let joined = read_invite_joined_rooms(conn, invitee, until_sn).await?;
    if joined.is_empty() {
        return Ok(Vec::new());
    }
    let rows = room_users::table
        .inner_join(rooms::table.on(room_users::room_id.eq(rooms::id)))
        .filter(room_users::user_id.eq_any(inviters))
        .filter(room_users::room_id.eq_any(joined))
        .distinct_on((room_users::user_id, room_users::room_id))
        .order_by((
            room_users::user_id.desc(),
            room_users::room_id.desc(),
            room_users::id.desc(),
        ))
        .select((
            room_users::user_id,
            room_users::membership,
            room_users::event_sn,
            room_users::room_id,
            rooms::state_frame_id,
        ))
        .load::<(OwnedUserId, String, i64, OwnedRoomId, Option<i64>)>(conn)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|(_, membership, event_sn, ..)| membership == "join" && *event_sn <= until_sn)
        .map(|(inviter, _, _, room, frame)| (inviter, room, frame))
        .collect())
}

async fn read_shared_invite_rooms(
    conn: &mut diesel_async::AsyncPgConnection,
    invitee: &UserId,
    inviters: &[OwnedUserId],
    until_sn: i64,
) -> DataResult<Vec<SharedInviteRoom>> {
    Ok(
        read_shared_invite_memberships(conn, invitee, inviters, until_sn)
            .await?
            .into_iter()
            .filter_map(|(inviter, _, frame)| {
                frame.map(|frame_id| SharedInviteRoom { inviter, frame_id })
            })
            .collect(),
    )
}

/// New-invite authorization uses one permission/membership/room-frame snapshot.
pub async fn invite_permission_snapshot(
    invitee: &UserId,
    inviters: &[OwnedUserId],
    membership_action: Option<&str>,
) -> DataResult<InvitePermissionSnapshot> {
    connect()
        .await?
        .build_transaction()
        .read_only()
        .repeatable_read()
        .run::<_, DataError, _>(async |conn| {
            let config = read_invite_permission(conn, invitee).await?;
            let default_action = invite_default_action(config.as_ref());
            let shared_rooms =
                if membership_action.is_some() && default_action.as_deref() == membership_action {
                    read_shared_invite_rooms(conn, invitee, inviters, i64::MAX).await?
                } else {
                    Vec::new()
                };
            Ok(InvitePermissionSnapshot {
                default_action,
                shared_rooms,
            })
        })
        .await
}

pub struct InviteSyncInventory {
    pub default_action: Option<String>,
    pub replay_since_sn: i64,
    pub invites: Vec<SyncInvitation>,
    pub shared_rooms: Vec<SharedInviteRoom>,
}

// Domain seeds separate user and room locks. Use bigint keys so these scopes
// cannot overlap account-data locks in PostgreSQL's separate two-int key space.
const INVITE_USER_LOCK: i64 = 0x494e5655;
const INVITE_ROOM_LOCK: i64 = 0x494e5652;

async fn lock_invite_write_scope(
    conn: &mut diesel_async::AsyncPgConnection,
    namespace: i64,
    id: &str,
) -> DataResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, $2))")
        .bind::<diesel::sql_types::Text, _>(id)
        .bind::<diesel::sql_types::BigInt, _>(namespace)
        .execute(conn)
        .await?;
    Ok(())
}

async fn lock_invite_read_scopes(
    conn: &mut diesel_async::AsyncPgConnection,
    namespace: i64,
    ids: Vec<String>,
) -> DataResult<()> {
    #[derive(QueryableByName)]
    struct ScopeKey {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        key: i64,
    }
    // Sort the actual PostgreSQL keys, rather than identifiers: hash collisions
    // must not give overlapping requests different physical lock orders.
    let keys = diesel::sql_query(
        "SELECT DISTINCT hashtextextended(id, $2) AS key FROM unnest($1::text[]) AS scopes(id) ORDER BY key",
    )
    .bind::<diesel::sql_types::Array<diesel::sql_types::Text>, _>(ids)
    .bind::<diesel::sql_types::BigInt, _>(namespace)
    .load::<ScopeKey>(conn)
    .await?;
    for key in keys {
        diesel::sql_query("SELECT pg_advisory_xact_lock_shared($1)")
            .bind::<diesel::sql_types::BigInt, _>(key.key)
            .execute(conn)
            .await?;
    }
    Ok(())
}

/// All builds coordinate user policy, ignore-list, membership and admission writes,
/// so an MSC4494 reader is also protected from a feature-disabled instance. Writers
/// hold exactly one user scope until commit and never acquire sync stream locks.
pub async fn lock_invite_user_write(
    conn: &mut diesel_async::AsyncPgConnection,
    user: &UserId,
) -> DataResult<()> {
    lock_invite_write_scope(conn, INVITE_USER_LOCK, user.as_str()).await
}

/// Current room-frame publication holds one room scope until commit, independently
/// of user-scoped membership transactions. Do not acquire user or stream locks here.
pub async fn lock_invite_room_write(
    conn: &mut diesel_async::AsyncPgConnection,
    room: &RoomId,
) -> DataResult<()> {
    lock_invite_write_scope(conn, INVITE_ROOM_LOCK, room.as_str()).await
}

async fn unadmitted_invite_senders(
    conn: &mut diesel_async::AsyncPgConnection,
    user: &UserId,
) -> DataResult<std::collections::BTreeSet<OwnedUserId>> {
    Ok(room_users::table
        .left_join(
            room_invite_admissions::table
                .on(room_invite_admissions::room_user_id.eq(room_users::id)),
        )
        .filter(room_users::user_id.eq(user))
        .filter(room_users::membership.eq("invite"))
        .filter(room_invite_admissions::room_user_id.is_null())
        .select(room_users::sender_id)
        .distinct()
        .load::<OwnedUserId>(conn)
        .await?
        .into_iter()
        .collect())
}

/// Take after the sync stream locks in READ COMMITTED. Discover users first and
/// acquire every user scope in physical-key order, including the recipient. Even shared
/// locks need one order when writers are queued. The recipient lock then freezes
/// policy, ignores, invitations and admissions. If a new sender appeared before
/// that lock, return false: the caller must release the transaction and retry.
/// After user scopes are fixed, freeze the shared rooms in sorted order, including
/// rooms without a frame. Readers overlap; unrelated users/rooms keep writing.
pub async fn lock_invite_sync_state(
    conn: &mut diesel_async::AsyncPgConnection,
    user: &UserId,
    membership_action: &str,
) -> DataResult<bool> {
    let preflight = read_invite_permission(conn, user).await?;
    let mut users =
        if invite_default_action(preflight.as_ref()).as_deref() == Some(membership_action) {
            unadmitted_invite_senders(conn, user).await?
        } else {
            std::collections::BTreeSet::new()
        };
    users.insert(user.to_owned());
    lock_invite_read_scopes(
        conn,
        INVITE_USER_LOCK,
        users.iter().map(ToString::to_string).collect(),
    )
    .await?;
    let config = read_invite_permission(conn, user).await?;
    if invite_default_action(config.as_ref()).as_deref() != Some(membership_action) {
        return Ok(true);
    }
    let inviters = unadmitted_invite_senders(conn, user).await?;
    if !inviters.is_subset(&users) {
        return Ok(false);
    }
    let shared = read_shared_invite_memberships(
        conn,
        user,
        &inviters.into_iter().collect::<Vec<_>>(),
        i64::MAX,
    )
    .await?;
    let shared_rooms: std::collections::BTreeSet<_> =
        shared.into_iter().map(|(_, room, _)| room).collect();
    lock_invite_read_scopes(
        conn,
        INVITE_ROOM_LOCK,
        shared_rooms
            .into_iter()
            .map(|room| room.to_string())
            .collect(),
    )
    .await?;
    Ok(true)
}

/// Read permission, ignored senders and invitations from one repeatable-read snapshot.
/// `retained_action` asks for older invites for an action whose eligibility is dynamic.
/// This query never advances the caller's stream boundary.
pub async fn invite_sync_inventory(
    user_id: &UserId,
    since_sn: i64,
    until_sn: i64,
    retained_action: Option<&str>,
    device_id: Option<&DeviceId>,
) -> DataResult<InviteSyncInventory> {
    connect()
        .await?
        .build_transaction()
        .read_only()
        .repeatable_read()
        .run::<_, DataError, _>(async |conn| {
            invite_sync_inventory_with_conn(
                conn,
                user_id,
                since_sn,
                until_sn,
                retained_action,
                device_id,
            )
            .await
        })
        .await
}

/// The caller supplies either a repeatable-read snapshot or an invitation-state
/// locks. The latter allow its protected sync cursor to be read on the same
/// connection without nested pool acquisitions or an unlocked sequence read.
pub async fn invite_sync_inventory_with_conn(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: &UserId,
    since_sn: i64,
    until_sn: i64,
    retained_action: Option<&str>,
    device_id: Option<&DeviceId>,
) -> DataResult<InviteSyncInventory> {
    let config = read_invite_permission(conn, user_id).await?;
    let default_action = invite_default_action(config.as_ref());
    let replay_since_sn = if config
        .as_ref()
        .is_some_and(|config| config.occur_sn >= since_sn)
    {
        0
    } else {
        since_sn
    };
    if default_action.as_deref() == Some("block") {
        return Ok(InviteSyncInventory {
            default_action,
            replay_since_sn,
            invites: Vec::new(),
            shared_rooms: Vec::new(),
        });
    }
    let load_since = if retained_action.is_some() && default_action.as_deref() == retained_action {
        0
    } else {
        replay_since_sn
    };
    let latest_membership = diesel::alias!(room_users as latest_invite_membership);
    let latest = latest_membership
        .filter(latest_membership.field(room_users::user_id).eq(user_id))
        .distinct_on(latest_membership.field(room_users::room_id))
        .order_by((
            latest_membership.field(room_users::room_id).desc(),
            latest_membership.field(room_users::id).desc(),
        ))
        .select(latest_membership.field(room_users::id));
    let delivered_sn = diesel::dsl::sql::<diesel::sql_types::Nullable<diesel::sql_types::BigInt>>(
        "(room_invite_admissions.delivered_devices ->> ",
    )
    .bind::<diesel::sql_types::Text, _>(device_id.map_or("", DeviceId::as_str))
    .sql(")::bigint");
    let mut query = room_users::table
        .left_join(
            room_invite_admissions::table
                .on(room_invite_admissions::room_user_id.eq(room_users::id)),
        )
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::id.eq_any(latest))
        .filter(room_users::membership.eq("invite"))
        .filter(room_users::event_sn.le(until_sn))
        .filter(
            room_users::sender_id.ne_all(
                user_ignores::table
                    .filter(user_ignores::user_id.eq(user_id))
                    .select(user_ignores::ignored_id),
            ),
        )
        .distinct_on(room_users::room_id)
        .order_by((room_users::room_id.desc(), room_users::id.desc()))
        .select((
            room_users::id,
            room_users::event_id,
            room_users::event_sn,
            room_users::room_id,
            room_users::sender_id,
            room_users::state_data,
            room_invite_admissions::admitted_sn.nullable(),
            delivered_sn.clone(),
        ))
        .into_boxed();
    // Tracking is enabled only with the membership-filtering feature. It also
    // recovers late/unseen allowed invites and deliveries on other devices,
    // even when their global admission was committed behind this cursor.
    query = if retained_action.is_some() {
        query.filter(
            room_users::event_sn
                .ge(load_since)
                .or(delivered_sn.clone().is_null())
                .or(delivered_sn.ge(since_sn)),
        )
    } else {
        query.filter(room_users::event_sn.ge(load_since))
    };
    let rows = query
        .load::<(
            i64,
            OwnedEventId,
            i64,
            OwnedRoomId,
            OwnedUserId,
            Option<JsonValue>,
            Option<i64>,
            Option<i64>,
        )>(conn)
        .await?;
    let invites: Vec<SyncInvitation> = rows
        .into_iter()
        .filter_map(
            |(
                membership_id,
                event_id,
                event_sn,
                room_id,
                sender_id,
                state,
                admitted_sn,
                delivered_sn,
            )| {
                state
                    .and_then(|state| serde_json::from_value(state).ok())
                    .map(|state| SyncInvitation {
                        membership_id,
                        event_id,
                        event_sn,
                        room_id,
                        sender_id,
                        state,
                        admitted_sn,
                        delivered_sn,
                    })
            },
        )
        .collect();
    let shared_rooms = if retained_action.is_some() && default_action.as_deref() == retained_action
    {
        let inviters: std::collections::HashSet<_> = invites
            .iter()
            .filter(|invite| invite.admitted_sn.is_none())
            .map(|invite| invite.sender_id.clone())
            .collect();
        read_shared_invite_rooms(
            conn,
            user_id,
            &inviters.into_iter().collect::<Vec<_>>(),
            until_sn,
        )
        .await?
    } else {
        Vec::new()
    };
    Ok(InviteSyncInventory {
        default_action,
        replay_since_sn,
        invites,
        shared_rooms,
    })
}

/// Stable permission filtering; membership reads and appservice delivery stay unchanged.
pub async fn invited_rooms_for_sync(
    user_id: &UserId,
    since_sn: i64,
) -> DataResult<Vec<(OwnedRoomId, Vec<RawJson<AnyStrippedStateEvent>>)>> {
    Ok(
        invite_sync_inventory(user_id, since_sn, i64::MAX, None, None)
            .await?
            .invites
            .into_iter()
            .map(|invite| (invite.room_id, invite.state))
            .collect(),
    )
}

/// Returns an iterator over all rooms a user was invited to.
pub async fn invited_rooms(
    user_id: &UserId,
    since_sn: i64,
) -> DataResult<Vec<(OwnedRoomId, Vec<RawJson<AnyStrippedStateEvent>>)>> {
    let ingored_ids = user_ignores::table
        .filter(user_ignores::user_id.eq(user_id))
        .select(user_ignores::ignored_id)
        .load::<OwnedUserId>(&mut connect().await?)
        .await?;
    let list = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::membership.eq("invite"))
        .filter(room_users::event_sn.ge(since_sn))
        .filter(room_users::sender_id.ne_all(&ingored_ids))
        .select((room_users::room_id, room_users::state_data))
        .load::<(OwnedRoomId, Option<JsonValue>)>(&mut connect().await?)
        .await?
        .into_iter()
        .filter_map(|(room_id, state_data)| {
            state_data
                .and_then(|state_data| serde_json::from_value(state_data).ok())
                .map(|state_data| (room_id, state_data))
        })
        .collect();
    Ok(list)
}

pub async fn knocked_rooms(
    user_id: &UserId,
    since_sn: i64,
) -> DataResult<Vec<(OwnedRoomId, Vec<RawJson<AnyStrippedStateEvent>>)>> {
    let list = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::membership.eq("knock"))
        .filter(room_users::event_sn.ge(since_sn))
        .select((room_users::room_id, room_users::state_data))
        .load::<(OwnedRoomId, Option<JsonValue>)>(&mut connect().await?)
        .await?
        .into_iter()
        .filter_map(|(room_id, state_data)| {
            state_data
                .and_then(|state_data| serde_json::from_value(state_data).ok())
                .map(|state_data| (room_id, state_data))
        })
        .collect();
    Ok(list)
}

/// Check if a user has an account on this homeserver.
pub async fn user_exists(user_id: &UserId) -> DataResult<bool> {
    let query = users::table.find(user_id);
    diesel_exists!(query, &mut connect().await?).map_err(Into::into)
}

pub async fn get_user(user_id: &UserId) -> DataResult<DbUser> {
    users::table
        .find(user_id)
        .first::<DbUser>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Insert a user, updating the existing row if one already exists.
pub async fn create_user(new_user: &NewDbUser) -> DataResult<DbUser> {
    diesel::insert_into(users::table)
        .values(new_user)
        .on_conflict(users::id)
        .do_update()
        .set(new_user)
        .get_result::<DbUser>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Mark a user account as deactivated without touching its other data.
pub async fn mark_deactivated(user_id: &UserId) -> DataResult<()> {
    // Evict before marking the account unusable so cached user state cannot
    // authenticate after the update commits but before the post-update scan runs.
    access_token::invalidate_user(user_id);
    diesel::update(users::table.find(user_id))
        .set(users::deactivated_at.eq(UnixMillis::now()))
        .execute(&mut connect().await?)
        .await?;
    access_token::invalidate_user(user_id);
    Ok(())
}

/// Returns the number of users registered on this server.
pub async fn count() -> DataResult<u64> {
    let count = user_passwords::table
        .select(dsl::count(user_passwords::user_id).aggregate_distinct())
        .first::<i64>(&mut connect().await?)
        .await?;
    Ok(count as u64)
}

/// Returns a list of local users as list of usernames.
///
/// A user account is considered `local` if the length of it's password is greater then zero.
pub async fn list_local_users() -> DataResult<Vec<OwnedUserId>> {
    user_passwords::table
        .select(user_passwords::user_id)
        .load::<OwnedUserId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Returns the display_name of a user on this homeserver.
pub async fn display_name(user_id: &UserId) -> DataResult<Option<String>> {
    user_profiles::table
        .filter(user_profiles::user_id.eq(user_id.as_str()))
        .filter(user_profiles::room_id.is_null())
        .select(user_profiles::display_name)
        .first::<Option<String>>(&mut connect().await?)
        .await
        .optional()
        .map(Option::flatten)
        .map_err(Into::into)
}
pub async fn set_display_name(user_id: &UserId, display_name: &str) -> DataResult<()> {
    profile::set_global_display_name(user_id, Some(display_name)).await
}
pub async fn remove_display_name(user_id: &UserId) -> DataResult<()> {
    profile::set_global_display_name(user_id, None).await
}

/// Get the avatar_url of a user.
pub async fn avatar_url(user_id: &UserId) -> DataResult<Option<OwnedMxcUri>> {
    user_profiles::table
        .filter(user_profiles::user_id.eq(user_id.as_str()))
        .filter(user_profiles::room_id.is_null())
        .select(user_profiles::avatar_url)
        .first::<Option<OwnedMxcUri>>(&mut connect().await?)
        .await
        .optional()
        .map(Option::flatten)
        .map_err(Into::into)
}
pub async fn set_avatar_url(user_id: &UserId, avatar_url: &MxcUri) -> DataResult<()> {
    profile::set_global_avatar_url(user_id, Some(avatar_url)).await
}
pub async fn remove_avatar_url(user_id: &UserId) -> DataResult<()> {
    profile::set_global_avatar_url(user_id, None).await
}

pub async fn delete_profile(user_id: &UserId) -> DataResult<()> {
    profile::delete_global_profile(user_id).await
}

/// Get the blurhash of a user.
pub async fn blurhash(user_id: &UserId) -> DataResult<Option<String>> {
    user_profiles::table
        .filter(user_profiles::user_id.eq(user_id.as_str()))
        .filter(user_profiles::room_id.is_null())
        .select(user_profiles::blurhash)
        .first::<Option<String>>(&mut connect().await?)
        .await
        .optional()
        .map(Option::flatten)
        .map_err(Into::into)
}

pub async fn is_deactivated(user_id: &UserId) -> DataResult<bool> {
    let deactivated_at = users::table
        .filter(users::id.eq(user_id))
        .select(users::deactivated_at)
        .first::<Option<UnixMillis>>(&mut connect().await?)
        .await
        .optional()?
        .flatten();
    Ok(deactivated_at.is_some())
}

pub async fn is_locked(user_id: &UserId) -> DataResult<bool> {
    let locked_at = users::table
        .filter(users::id.eq(user_id))
        .select(users::locked_at)
        .first::<Option<UnixMillis>>(&mut connect().await?)
        .await
        .optional()?
        .flatten();
    Ok(locked_at.is_some())
}

pub async fn is_suspended(user_id: &UserId) -> DataResult<bool> {
    let suspended_at = users::table
        .filter(users::id.eq(user_id))
        .select(users::suspended_at)
        .first::<Option<UnixMillis>>(&mut connect().await?)
        .await
        .optional()?
        .flatten();
    Ok(suspended_at.is_some())
}

pub async fn is_guest(user_id: &UserId) -> DataResult<bool> {
    users::table
        .filter(users::id.eq(user_id))
        .select(users::is_guest)
        .first::<bool>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

pub async fn set_guest(user_id: &UserId, is_guest: bool) -> DataResult<()> {
    diesel::update(users::table.find(user_id))
        .set(users::is_guest.eq(is_guest))
        .execute(&mut connect().await?)
        .await?;
    access_token::invalidate_user(user_id);
    Ok(())
}

pub async fn all_device_ids(user_id: &UserId) -> DataResult<Vec<OwnedDeviceId>> {
    user_devices::table
        .filter(user_devices::user_id.eq(user_id))
        .select(user_devices::device_id)
        .load::<OwnedDeviceId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Every device that can receive a to-device message, including the dehydrated device.
///
/// A dehydrated device is not in `user_devices`, but it is a device of the user as far as
/// senders are concerned: it appears in `/keys/query` and is meant to collect room keys
/// while the user has nothing else logged in. Leaving it out of a `*` fan-out would lose
/// exactly the messages dehydration exists to preserve ([MSC3814]).
///
/// [MSC3814]: https://github.com/matrix-org/matrix-spec-proposals/pull/3814
pub async fn all_to_device_target_ids(user_id: &UserId) -> DataResult<Vec<OwnedDeviceId>> {
    let mut device_ids = all_device_ids(user_id).await?;
    if let Some((dehydrated_id, _)) = get_dehydrated_device(user_id).await?
        && !device_ids.contains(&dehydrated_id)
    {
        device_ids.push(dehydrated_id);
    }
    Ok(device_ids)
}

pub async fn delete_access_tokens(user_id: &UserId) -> DataResult<()> {
    // Evict before the bulk revocation so cached tokens cannot keep
    // authenticating in the window before the post-delete scan runs.
    access_token::invalidate_user(user_id);
    diesel::delete(user_access_tokens::table.filter(user_access_tokens::user_id.eq(user_id)))
        .execute(&mut connect().await?)
        .await?;
    access_token::invalidate_user(user_id);
    Ok(())
}

pub async fn delete_refresh_tokens(user_id: &UserId) -> DataResult<()> {
    diesel::delete(user_refresh_tokens::table.filter(user_refresh_tokens::user_id.eq(user_id)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

pub async fn remove_all_devices(user_id: &UserId) -> DataResult<()> {
    for device in device::get_devices(user_id).await? {
        device::remove_device(user_id, &device.device_id).await?;
    }
    device::remove_all_user_to_device_events(user_id).await?;
    key::delete_all_device_keys(user_id).await?;
    delete_access_tokens(user_id).await?;
    delete_refresh_tokens(user_id).await?;
    pusher::delete_user_pushers(user_id).await?;
    delete_dehydrated_devices(user_id).await
}
pub async fn delete_dehydrated_devices(user_id: &UserId) -> DataResult<()> {
    let device_ids = user_dehydrated_devices::table
        .filter(user_dehydrated_devices::user_id.eq(user_id))
        .select(user_dehydrated_devices::device_id)
        .load::<OwnedDeviceId>(&mut connect().await?)
        .await?;

    for device_id in device_ids {
        key::delete_device_key_material(user_id, &device_id).await?;
        // The device is gone, so nothing will ever rehydrate it and read its inbox.
        // Without this the messages sit there for the lifetime of the account.
        //
        // Unless a live device now has the same ID -- logging in with an explicit device
        // ID does not consult the dehydrated table, so the two can collide, and the
        // inbox then belongs to the live device.
        device::remove_to_device_events_unless_live(user_id, &device_id).await?;
    }

    diesel::delete(
        user_dehydrated_devices::table.filter(user_dehydrated_devices::user_id.eq(user_id)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

pub async fn get_dehydrated_device(
    user_id: &UserId,
) -> DataResult<Option<(OwnedDeviceId, DehydratedDeviceData)>> {
    let Some((device_id, device_data)) = user_dehydrated_devices::table
        .filter(user_dehydrated_devices::user_id.eq(user_id))
        .order_by(user_dehydrated_devices::id.desc())
        .select((
            user_dehydrated_devices::device_id,
            user_dehydrated_devices::device_data,
        ))
        .first::<(OwnedDeviceId, JsonValue)>(&mut connect().await?)
        .await
        .optional()?
    else {
        return Ok(None);
    };

    Ok(Some((device_id, serde_json::from_value(device_data)?)))
}

pub async fn upsert_dehydrated_device(
    user_id: &UserId,
    device_id: &DeviceId,
    device_data: &DehydratedDeviceData,
) -> DataResult<()> {
    let current_device_id = user_dehydrated_devices::table
        .filter(user_dehydrated_devices::user_id.eq(user_id))
        .select(user_dehydrated_devices::device_id)
        .first::<OwnedDeviceId>(&mut connect().await?)
        .await
        .optional()?;

    if let Some(current_device_id) = current_device_id {
        key::delete_device_key_material(user_id, &current_device_id).await?;
        // Replacing the dehydrated device abandons the old one; its undelivered messages
        // can no longer be decrypted by anything and would leak. Unless the ID is also a
        // live device's, in which case the inbox is that device's, not the abandoned one's.
        if current_device_id != device_id {
            device::remove_to_device_events_unless_live(user_id, &current_device_id).await?;
        }
    }

    let new_device = NewDbUserDehydratedDevice {
        user_id: user_id.to_owned(),
        device_id: device_id.to_owned(),
        device_data: serde_json::to_value(device_data)?,
    };
    diesel::insert_into(user_dehydrated_devices::table)
        .values(&new_device)
        .on_conflict(user_dehydrated_devices::user_id)
        .do_update()
        .set((
            user_dehydrated_devices::device_id.eq(&new_device.device_id),
            user_dehydrated_devices::device_data.eq(&new_device.device_data),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Ensure that a user only sees signatures from themselves and the target user
pub fn clean_signatures<F: Fn(&UserId) -> bool>(
    cross_signing_key: &mut serde_json::Value,
    sender_id: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: F,
) -> DataResult<()> {
    if let Some(signatures) = cross_signing_key
        .get_mut("signatures")
        .and_then(|v| v.as_object_mut())
    {
        // Don't allocate for the full size of the current signatures, but require
        // at most one resize if nothing is dropped
        let new_capacity = signatures.len() / 2;
        for (user, signature) in
            mem::replace(signatures, serde_json::Map::with_capacity(new_capacity))
        {
            let sid = <&UserId>::try_from(user.as_str())
                .map_err(|_| DataError::internal("Invalid user ID in database."))?;
            // Keep a signature only if it was made by the requesting user, by the
            // target user themselves, or by an otherwise allowed origin. Comparing
            // against `Some(sid)` (rather than `Some(user_id)`) prevents a self-query
            // from leaking third-party cross-signing signatures stored over the
            // target's keys.
            if sender_id == Some(sid) || sid == user_id || allowed_signatures(sid) {
                signatures.insert(user, signature);
            }
        }
    }

    Ok(())
}

pub async fn deactivate(user_id: &UserId) -> DataResult<()> {
    // Evict before the state change so a cache hit can't keep serving the
    // still-usable account during the deletes below; invalidated again at the
    // end to drop anything a concurrent lookup re-populated.
    access_token::invalidate_user(user_id);
    diesel::update(users::table.find(user_id))
        .set((users::deactivated_at.eq(UnixMillis::now()),))
        .execute(&mut connect().await?)
        .await?;

    diesel::delete(user_threepids::table.filter(user_threepids::user_id.eq(user_id)))
        .execute(&mut connect().await?)
        .await?;
    diesel::delete(user_access_tokens::table.filter(user_access_tokens::user_id.eq(user_id)))
        .execute(&mut connect().await?)
        .await?;
    access_token::invalidate_user(user_id);

    Ok(())
}

pub async fn reactivate(user_id: &UserId) -> DataResult<()> {
    diesel::update(users::table.find(user_id))
        .set(users::deactivated_at.eq::<Option<UnixMillis>>(None))
        .execute(&mut connect().await?)
        .await?;
    access_token::invalidate_user(user_id);
    Ok(())
}

pub async fn set_ignored_users(user_id: &UserId, ignored_ids: &[OwnedUserId]) -> DataResult<()> {
    connect()
        .await?
        .transaction::<_, DataError, _>(async |conn| {
            lock_invite_user_write(conn, user_id).await?;
            diesel::delete(user_ignores::table.filter(user_ignores::user_id.eq(user_id)))
                .execute(conn)
                .await?;
            for ignored_id in ignored_ids {
                diesel::insert_into(user_ignores::table)
                    .values(NewDbUserIgnore {
                        user_id: user_id.to_owned(),
                        ignored_id: ignored_id.to_owned(),
                        created_at: UnixMillis::now(),
                    })
                    .on_conflict_do_nothing()
                    .execute(conn)
                    .await?;
            }
            Ok(())
        })
        .await
}

/// Get user_id by third party ID (email, phone, etc.)
pub async fn get_user_by_threepid(medium: &str, address: &str) -> DataResult<Option<OwnedUserId>> {
    user_threepids::table
        .filter(user_threepids::medium.eq(medium))
        .filter(user_threepids::address.eq(address))
        .select(user_threepids::user_id)
        .first::<OwnedUserId>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Threepid info for admin API
#[derive(Debug, Clone)]
pub struct ThreepidInfo {
    pub medium: String,
    pub address: String,
    pub added_at: UnixMillis,
    pub validated_at: UnixMillis,
}

/// Get all threepids for a user
pub async fn get_threepids(user_id: &UserId) -> DataResult<Vec<ThreepidInfo>> {
    user_threepids::table
        .filter(user_threepids::user_id.eq(user_id))
        .select((
            user_threepids::medium,
            user_threepids::address,
            user_threepids::added_at,
            user_threepids::validated_at,
        ))
        .load::<(String, String, UnixMillis, UnixMillis)>(&mut connect().await?)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|(medium, address, added_at, validated_at)| ThreepidInfo {
                    medium,
                    address,
                    added_at,
                    validated_at,
                })
                .collect()
        })
        .map_err(Into::into)
}

/// Replace all threepids for a user
pub async fn replace_threepids(
    user_id: &UserId,
    threepids: &[(String, String, Option<i64>, Option<i64>)],
) -> DataResult<()> {
    let mut conn = connect().await?;
    diesel::delete(user_threepids::table.filter(user_threepids::user_id.eq(user_id)))
        .execute(&mut conn)
        .await?;

    let now = UnixMillis::now();
    for (medium, address, added_at, validated_at) in threepids {
        diesel::insert_into(user_threepids::table)
            .values(NewDbUserThreepid {
                user_id: user_id.to_owned(),
                medium: medium.clone(),
                address: address.clone(),
                validated_at: validated_at.map(|ts| UnixMillis(ts as u64)).unwrap_or(now),
                added_at: added_at.map(|ts| UnixMillis(ts as u64)).unwrap_or(now),
            })
            .execute(&mut conn)
            .await?;
    }
    Ok(())
}

/// Set admin status for a user
pub async fn set_admin(user_id: &UserId, is_admin: bool) -> DataResult<()> {
    access_token::invalidate_user(user_id);
    diesel::update(users::table.find(user_id))
        .set(users::is_admin.eq(is_admin))
        .execute(&mut connect().await?)
        .await?;
    access_token::invalidate_user(user_id);
    Ok(())
}

/// Set shadow ban status for a user
pub async fn set_shadow_banned(user_id: &UserId, shadow_banned: bool) -> DataResult<()> {
    diesel::update(users::table.find(user_id))
        .set(users::shadow_banned.eq(shadow_banned))
        .execute(&mut connect().await?)
        .await?;
    access_token::invalidate_user(user_id);
    Ok(())
}

/// Set user type (e.g. guest/user/admin specific types)
pub async fn set_user_type(user_id: &UserId, user_type: Option<&str>) -> DataResult<()> {
    diesel::update(users::table.find(user_id))
        .set(users::ty.eq(user_type))
        .execute(&mut connect().await?)
        .await?;
    access_token::invalidate_user(user_id);
    Ok(())
}

/// Set locked status for a user
pub async fn set_locked(
    user_id: &UserId,
    locked: bool,
    locker_id: Option<&UserId>,
) -> DataResult<()> {
    // Evict before changing account usability so cached user state cannot
    // authenticate after the update commits but before the post-update scan runs.
    access_token::invalidate_user(user_id);
    if locked {
        diesel::update(users::table.find(user_id))
            .set((
                users::locked_at.eq(Some(UnixMillis::now())),
                users::locked_by.eq(locker_id.map(|u| u.to_owned())),
            ))
            .execute(&mut connect().await?)
            .await?;
    } else {
        diesel::update(users::table.find(user_id))
            .set((
                users::locked_at.eq::<Option<UnixMillis>>(None),
                users::locked_by.eq::<Option<OwnedUserId>>(None),
            ))
            .execute(&mut connect().await?)
            .await?;
    }
    access_token::invalidate_user(user_id);
    Ok(())
}

/// Set suspended status for a user
pub async fn set_suspended(user_id: &UserId, suspended: bool) -> DataResult<()> {
    // Evict before changing account usability so cached user state cannot
    // authenticate after the update commits but before the post-update scan runs.
    access_token::invalidate_user(user_id);
    if suspended {
        diesel::update(users::table.find(user_id))
            .set(users::suspended_at.eq(Some(UnixMillis::now())))
            .execute(&mut connect().await?)
            .await?;
    } else {
        diesel::update(users::table.find(user_id))
            .set(users::suspended_at.eq::<Option<UnixMillis>>(None))
            .execute(&mut connect().await?)
            .await?;
    }
    access_token::invalidate_user(user_id);
    Ok(())
}

/// List users with pagination and filtering
#[derive(Debug, Clone, Default)]
pub struct ListUsersFilter {
    /// Only list accounts on this server name. The admin API sets it to the
    /// local server name so rows for other servers never show up as accounts.
    pub server_name: Option<String>,
    pub from: Option<i64>,
    pub limit: Option<i64>,
    pub name: Option<String>,
    pub guests: Option<bool>,
    pub deactivated: Option<bool>,
    pub admins: Option<bool>,
    pub user_types: Option<Vec<String>>,
    pub order_by: Option<String>,
    pub dir: Option<String>,
}

/// Escape special characters in SQL LIKE/ILIKE patterns.
///
/// This prevents users from injecting wildcard characters (% and _)
/// that could modify the query behavior.
fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() * 2);
    for c in value.chars() {
        match c {
            '%' => escaped.push_str("\\%"),
            '_' => escaped.push_str("\\_"),
            '\\' => escaped.push_str("\\\\"),
            _ => escaped.push(c),
        }
    }
    escaped
}

pub async fn list_users(filter: &ListUsersFilter) -> DataResult<(Vec<DbUser>, i64)> {
    let mut query = users::table.into_boxed();
    let mut count_query = users::table.into_boxed();

    if let Some(ref server_name) = filter.server_name {
        query = query.filter(users::server_name.eq(server_name.clone()));
        count_query = count_query.filter(users::server_name.eq(server_name.clone()));
    }

    // Filter by name (localpart contains)
    // Escape LIKE wildcards to prevent pattern injection attacks
    if let Some(ref name) = filter.name {
        let escaped_name = escape_like_pattern(name);
        let pattern = format!("%{}%", escaped_name);
        query = query.filter(users::localpart.ilike(pattern.clone()));
        count_query = count_query.filter(users::localpart.ilike(pattern));
    }

    // Filter by guests
    if let Some(guests) = filter.guests {
        query = query.filter(users::is_guest.eq(guests));
        count_query = count_query.filter(users::is_guest.eq(guests));
    }

    // Filter by deactivated
    if let Some(deactivated) = filter.deactivated {
        if deactivated {
            query = query.filter(users::deactivated_at.is_not_null());
            count_query = count_query.filter(users::deactivated_at.is_not_null());
        } else {
            query = query.filter(users::deactivated_at.is_null());
            count_query = count_query.filter(users::deactivated_at.is_null());
        }
    }

    // Filter by admin
    if let Some(admins) = filter.admins {
        query = query.filter(users::is_admin.eq(admins));
        count_query = count_query.filter(users::is_admin.eq(admins));
    }

    // Get total count with filters applied
    let total: i64 = count_query
        .count()
        .get_result(&mut connect().await?)
        .await?;

    // Apply ordering
    let dir_asc = filter.dir.as_ref().map(|d| d == "f").unwrap_or(true);
    query = match filter.order_by.as_deref() {
        Some("name") => {
            if dir_asc {
                query.order(users::localpart.asc())
            } else {
                query.order(users::localpart.desc())
            }
        }
        Some("is_guest") => {
            if dir_asc {
                query.order(users::is_guest.asc())
            } else {
                query.order(users::is_guest.desc())
            }
        }
        Some("admin") => {
            if dir_asc {
                query.order(users::is_admin.asc())
            } else {
                query.order(users::is_admin.desc())
            }
        }
        Some("deactivated") => {
            if dir_asc {
                query.order(users::deactivated_at.asc())
            } else {
                query.order(users::deactivated_at.desc())
            }
        }
        Some("shadow_banned") => {
            if dir_asc {
                query.order(users::shadow_banned.asc())
            } else {
                query.order(users::shadow_banned.desc())
            }
        }
        _ => {
            if dir_asc {
                query.order(users::created_at.asc())
            } else {
                query.order(users::created_at.desc())
            }
        }
    };

    // Apply pagination
    if let Some(from) = filter.from {
        query = query.offset(from);
    }

    let limit = filter.limit.unwrap_or(100).min(1000);
    query = query.limit(limit);

    let users = query.load::<DbUser>(&mut connect().await?).await?;

    Ok((users, total))
}

/// Ratelimit override info
#[derive(Debug, Clone)]
pub struct RateLimitOverride {
    pub messages_per_second: Option<i32>,
    pub burst_count: Option<i32>,
}

pub async fn get_ratelimit(user_id: &UserId) -> DataResult<Option<RateLimitOverride>> {
    user_ratelimit_override::table
        .find(user_id)
        .select((
            user_ratelimit_override::messages_per_second,
            user_ratelimit_override::burst_count,
        ))
        .first::<(Option<i32>, Option<i32>)>(&mut connect().await?)
        .await
        .optional()
        .map(|opt| {
            opt.map(|(mps, bc)| RateLimitOverride {
                messages_per_second: mps,
                burst_count: bc,
            })
        })
        .map_err(Into::into)
}

pub async fn set_ratelimit(
    user_id: &UserId,
    messages_per_second: Option<i32>,
    burst_count: Option<i32>,
) -> DataResult<()> {
    diesel::insert_into(user_ratelimit_override::table)
        .values((
            user_ratelimit_override::user_id.eq(user_id),
            user_ratelimit_override::messages_per_second.eq(messages_per_second),
            user_ratelimit_override::burst_count.eq(burst_count),
        ))
        .on_conflict(user_ratelimit_override::user_id)
        .do_update()
        .set((
            user_ratelimit_override::messages_per_second.eq(messages_per_second),
            user_ratelimit_override::burst_count.eq(burst_count),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

pub async fn delete_ratelimit(user_id: &UserId) -> DataResult<()> {
    diesel::delete(user_ratelimit_override::table.find(user_id))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}
