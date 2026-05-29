//! Persistence for MSC2444 federated peeking.
//!
//! Two independent concerns:
//! - `room_peeking_servers`: remote servers peeking one of *our* rooms (resident
//!   side). New events are delivered to them until their peek expires/cancels.
//! - `room_peeks`: outbound peeks *we* hold on remote rooms (peeking side),
//!   renewed before `renew_at`.

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::identifiers::*;
use crate::core::UnixMillis;
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = room_peeking_servers)]
pub struct DbPeekingServer {
    pub id: i64,
    pub room_id: OwnedRoomId,
    pub server_id: OwnedServerName,
    pub peek_id: String,
    pub renew_at: i64,
    pub created_at: i64,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = room_peeks)]
pub struct DbRoomPeek {
    pub id: i64,
    pub room_id: OwnedRoomId,
    pub peek_id: String,
    pub target_server: OwnedServerName,
    pub renew_at: i64,
    pub created_at: i64,
}

// ---------------------------------------------------------------------------
// Resident side: remote servers peeking our rooms.
// ---------------------------------------------------------------------------

/// Register or renew a remote server's peek on one of our rooms. `renew_at` is
/// the unix-millis deadline by which the peer must renew or the peek lapses.
pub async fn upsert_peeking_server(
    room_id: &RoomId,
    server: &ServerName,
    peek_id: &str,
    renew_at: i64,
) -> DataResult<()> {
    let now = UnixMillis::now().get() as i64;
    diesel::insert_into(room_peeking_servers::table)
        .values((
            room_peeking_servers::room_id.eq(room_id),
            room_peeking_servers::server_id.eq(server),
            room_peeking_servers::peek_id.eq(peek_id),
            room_peeking_servers::renew_at.eq(renew_at),
            room_peeking_servers::created_at.eq(now),
        ))
        .on_conflict((
            room_peeking_servers::room_id,
            room_peeking_servers::server_id,
            room_peeking_servers::peek_id,
        ))
        .do_update()
        .set(room_peeking_servers::renew_at.eq(renew_at))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Cancel a specific peek a remote server holds on one of our rooms.
pub async fn remove_peeking_server(
    room_id: &RoomId,
    server: &ServerName,
    peek_id: &str,
) -> DataResult<()> {
    diesel::delete(
        room_peeking_servers::table
            .filter(room_peeking_servers::room_id.eq(room_id))
            .filter(room_peeking_servers::server_id.eq(server))
            .filter(room_peeking_servers::peek_id.eq(peek_id)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Distinct servers with a non-expired peek on the given room (`renew_at` in the
/// future relative to `now_ms`). These are extra destinations for room events.
pub async fn active_peeking_servers(
    room_id: &RoomId,
    now_ms: i64,
) -> DataResult<Vec<OwnedServerName>> {
    let mut servers = room_peeking_servers::table
        .filter(room_peeking_servers::room_id.eq(room_id))
        .filter(room_peeking_servers::renew_at.gt(now_ms))
        .select(room_peeking_servers::server_id)
        .load::<OwnedServerName>(&mut connect().await?)
        .await?;
    servers.sort_unstable();
    servers.dedup();
    Ok(servers)
}

/// Delete peeks (held by remote servers on our rooms) that lapsed before
/// `now_ms`. Returns the number of rows removed.
pub async fn purge_expired_peeking_servers(now_ms: i64) -> DataResult<usize> {
    let removed = diesel::delete(
        room_peeking_servers::table.filter(room_peeking_servers::renew_at.le(now_ms)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Peeking side: outbound peeks we hold on remote rooms.
// ---------------------------------------------------------------------------

/// Record or refresh our outbound peek on a remote room (one per room).
pub async fn upsert_peek(
    room_id: &RoomId,
    peek_id: &str,
    target_server: &ServerName,
    renew_at: i64,
) -> DataResult<()> {
    let now = UnixMillis::now().get() as i64;
    diesel::insert_into(room_peeks::table)
        .values((
            room_peeks::room_id.eq(room_id),
            room_peeks::peek_id.eq(peek_id),
            room_peeks::target_server.eq(target_server),
            room_peeks::renew_at.eq(renew_at),
            room_peeks::created_at.eq(now),
        ))
        .on_conflict(room_peeks::room_id)
        .do_update()
        .set((
            room_peeks::peek_id.eq(peek_id),
            room_peeks::target_server.eq(target_server),
            room_peeks::renew_at.eq(renew_at),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Our active outbound peek on a room, if any.
pub async fn get_peek(room_id: &RoomId) -> DataResult<Option<DbRoomPeek>> {
    let peek = room_peeks::table
        .filter(room_peeks::room_id.eq(room_id))
        .first::<DbRoomPeek>(&mut connect().await?)
        .await
        .optional()?;
    Ok(peek)
}

/// Whether we currently hold an outbound peek on the room.
pub async fn is_peeked(room_id: &RoomId) -> DataResult<bool> {
    Ok(get_peek(room_id).await?.is_some())
}

/// Drop our outbound peek on a room.
pub async fn remove_peek(room_id: &RoomId) -> DataResult<()> {
    diesel::delete(room_peeks::table.filter(room_peeks::room_id.eq(room_id)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Outbound peeks whose renewal deadline is at or before `now_ms` — these need
/// to be renewed (or dropped if the remote refuses).
pub async fn peeks_due_for_renewal(now_ms: i64) -> DataResult<Vec<DbRoomPeek>> {
    let peeks = room_peeks::table
        .filter(room_peeks::renew_at.le(now_ms))
        .load::<DbRoomPeek>(&mut connect().await?)
        .await?;
    Ok(peeks)
}

/// All rooms we currently peek (for sync inclusion).
pub async fn peeked_room_ids() -> DataResult<Vec<OwnedRoomId>> {
    let rooms = room_peeks::table
        .select(room_peeks::room_id)
        .load::<OwnedRoomId>(&mut connect().await?)
        .await?;
    Ok(rooms)
}

// ---------------------------------------------------------------------------
// MSC2753 client-side peeking: which local users peek which rooms.
// ---------------------------------------------------------------------------

/// Record that a local user is peeking a room (idempotent).
pub async fn add_user_peek(user_id: &UserId, room_id: &RoomId) -> DataResult<()> {
    let now = UnixMillis::now().get() as i64;
    diesel::insert_into(user_peeks::table)
        .values((
            user_peeks::user_id.eq(user_id),
            user_peeks::room_id.eq(room_id),
            user_peeks::created_at.eq(now),
        ))
        .on_conflict_do_nothing()
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Stop a local user peeking a room.
pub async fn remove_user_peek(user_id: &UserId, room_id: &RoomId) -> DataResult<()> {
    diesel::delete(
        user_peeks::table
            .filter(user_peeks::user_id.eq(user_id))
            .filter(user_peeks::room_id.eq(room_id)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Rooms a local user is currently peeking.
pub async fn user_peeked_rooms(user_id: &UserId) -> DataResult<Vec<OwnedRoomId>> {
    let rooms = user_peeks::table
        .filter(user_peeks::user_id.eq(user_id))
        .select(user_peeks::room_id)
        .load::<OwnedRoomId>(&mut connect().await?)
        .await?;
    Ok(rooms)
}

/// Number of local users currently peeking a room.
pub async fn room_peeker_count(room_id: &RoomId) -> DataResult<i64> {
    let count: i64 = user_peeks::table
        .filter(user_peeks::room_id.eq(room_id))
        .count()
        .get_result(&mut connect().await?)
        .await?;
    Ok(count)
}
