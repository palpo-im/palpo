use std::fmt::Debug;

use diesel::prelude::*;

use crate::core::identifiers::*;
pub use crate::core::sending::*;
use crate::schema::*;
use crate::{DataResult, connect};

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
pub fn get_all_destinations() -> DataResult<Vec<OwnedServerName>> {
    let servers: Vec<OwnedServerName> = outgoing_requests::table
        .filter(outgoing_requests::server_id.is_not_null())
        .select(outgoing_requests::server_id)
        .distinct()
        .load::<Option<OwnedServerName>>(&mut connect()?)?
        .into_iter()
        .flatten()
        .collect();
    Ok(servers)
}

/// Check if a destination is known
pub fn is_destination_known(server: &ServerName) -> DataResult<bool> {
    let query = outgoing_requests::table
        .filter(outgoing_requests::server_id.eq(server));
    Ok(diesel_exists!(query, &mut connect()?)?)
}

/// Get rooms shared with a destination
pub fn get_destination_rooms(server: &ServerName) -> DataResult<Vec<OwnedRoomId>> {
    use crate::schema::room_joined_servers;
    let rooms: Vec<OwnedRoomId> = room_joined_servers::table
        .filter(room_joined_servers::server_id.eq(server))
        .select(room_joined_servers::room_id)
        .load(&mut connect()?)?;
    Ok(rooms)
}

/// Reset retry timings for a destination
pub fn reset_destination_retry(_server: &ServerName) -> DataResult<()> {
    // TODO: Implement retry timing reset when retry tracking is added
    Ok(())
}
