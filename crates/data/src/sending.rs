use std::fmt::Debug;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::identifiers::*;
pub use crate::core::sending::*;
use crate::core::UnixMillis;
use crate::schema::*;
use crate::{DataResult, connect};

/// Selector identifying which active outgoing-request rows a retry-state update
/// applies to. Mirrors the variants of the server-side `OutgoingKind`.
pub enum OutgoingDestination<'a> {
    Normal(&'a ServerName),
    Appservice(&'a str),
    Push { user_id: &'a UserId, pushkey: &'a str },
}

#[derive(Identifiable, Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = outgoing_requests)]
pub struct DbOutgoingRequest {
    pub id: i64,
    pub kind: String,
    pub appservice_id: Option<String>,
    pub user_id: Option<OwnedUserId>,
    pub pushkey: Option<String>,
    pub server_id: Option<OwnedServerName>,
    pub pdu_id: Option<OwnedEventId>,
    pub edu_json: Option<Vec<u8>>,
    pub state: String,
    pub data: Option<Vec<u8>>,
    pub retry_count: i32,
    pub last_failed_at: Option<i64>,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = outgoing_requests)]
pub struct NewDbOutgoingRequest {
    pub kind: String,
    pub appservice_id: Option<String>,
    pub user_id: Option<OwnedUserId>,
    pub pushkey: Option<String>,
    pub server_id: Option<OwnedServerName>,
    pub pdu_id: Option<OwnedEventId>,
    pub edu_json: Option<Vec<u8>>,
}

/// Get all known federation destinations
pub async fn get_all_destinations() -> DataResult<Vec<OwnedServerName>> {
    let servers: Vec<OwnedServerName> = outgoing_requests::table
        .filter(outgoing_requests::server_id.is_not_null())
        .select(outgoing_requests::server_id)
        .distinct()
        .load::<Option<OwnedServerName>>(&mut connect().await?)
        .await?
        .into_iter()
        .flatten()
        .collect();
    Ok(servers)
}

/// Check if a destination is known
pub async fn is_destination_known(server: &ServerName) -> DataResult<bool> {
    let query = outgoing_requests::table.filter(outgoing_requests::server_id.eq(server));
    Ok(diesel_exists!(query, &mut connect().await?)?)
}

/// Get rooms shared with a destination
pub async fn get_destination_rooms(server: &ServerName) -> DataResult<Vec<OwnedRoomId>> {
    use crate::schema::room_joined_servers;
    let rooms: Vec<OwnedRoomId> = room_joined_servers::table
        .filter(room_joined_servers::server_id.eq(server))
        .select(room_joined_servers::room_id)
        .load(&mut connect().await?)
        .await?;
    Ok(rooms)
}

/// Persist retry state for the active outgoing requests of a destination so
/// other instances can respect the same backoff window.
pub async fn persist_retry_state(
    dest: OutgoingDestination<'_>,
    tries: u32,
) -> DataResult<()> {
    let now = UnixMillis::now().get() as i64;
    match dest {
        OutgoingDestination::Normal(server_name) => {
            diesel::update(
                outgoing_requests::table
                    .filter(outgoing_requests::kind.eq("normal"))
                    .filter(outgoing_requests::server_id.eq(server_name))
                    .filter(outgoing_requests::state.eq("active")),
            )
            .set((
                outgoing_requests::retry_count.eq(tries as i32),
                outgoing_requests::last_failed_at.eq(Some(now)),
            ))
            .execute(&mut connect().await?)
            .await?;
        }
        OutgoingDestination::Appservice(id) => {
            diesel::update(
                outgoing_requests::table
                    .filter(outgoing_requests::kind.eq("appservice"))
                    .filter(outgoing_requests::appservice_id.eq(id))
                    .filter(outgoing_requests::state.eq("active")),
            )
            .set((
                outgoing_requests::retry_count.eq(tries as i32),
                outgoing_requests::last_failed_at.eq(Some(now)),
            ))
            .execute(&mut connect().await?)
            .await?;
        }
        OutgoingDestination::Push { user_id, pushkey } => {
            diesel::update(
                outgoing_requests::table
                    .filter(outgoing_requests::kind.eq("push"))
                    .filter(outgoing_requests::user_id.eq(user_id))
                    .filter(outgoing_requests::pushkey.eq(pushkey))
                    .filter(outgoing_requests::state.eq("active")),
            )
            .set((
                outgoing_requests::retry_count.eq(tries as i32),
                outgoing_requests::last_failed_at.eq(Some(now)),
            ))
            .execute(&mut connect().await?)
            .await?;
        }
    }
    Ok(())
}

/// Reset retry timings for a destination
pub async fn reset_destination_retry(server: &ServerName) -> DataResult<()> {
    diesel::update(
        outgoing_requests::table
            .filter(outgoing_requests::kind.eq("normal"))
            .filter(outgoing_requests::server_id.eq(server.as_str())),
    )
    .set((
        outgoing_requests::retry_count.eq(0),
        outgoing_requests::last_failed_at.eq(None::<i64>),
    ))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}
