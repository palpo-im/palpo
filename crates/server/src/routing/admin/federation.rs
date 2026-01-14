//! Admin Federation API
//!
//! - GET /_synapse/admin/v1/federation/destinations
//! - GET /_synapse/admin/v1/federation/destinations/{destination}
//! - GET /_synapse/admin/v1/federation/destinations/{destination}/rooms
//! - POST /_synapse/admin/v1/federation/destinations/{destination}/reset_connection

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::core::identifiers::*;
use crate::{JsonResult, MatrixError, data, json_ok};

pub fn router() -> Router {
    Router::new().push(
        Router::with_path("v1/federation/destinations")
            .get(list_destinations)
            .push(
                Router::with_path("{destination}")
                    .get(get_destination)
                    .push(Router::with_path("rooms").get(destination_rooms))
                    .push(Router::with_path("reset_connection").post(reset_connection)),
            ),
    )
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DestinationInfo {
    pub destination: String,
    pub retry_last_ts: i64,
    pub retry_interval: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_ts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_successful_stream_ordering: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DestinationsListResponse {
    pub destinations: Vec<DestinationInfo>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DestinationRoomsResponse {
    pub rooms: Vec<DestinationRoom>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DestinationRoom {
    pub room_id: String,
    pub stream_ordering: i64,
}

/// GET /_synapse/admin/v1/federation/destinations
///
/// List all federation destinations
#[endpoint]
pub fn list_destinations(
    from: QueryParam<i64, false>,
    limit: QueryParam<i64, false>,
    _destination: QueryParam<String, false>,
    _order_by: QueryParam<String, false>,
    _dir: QueryParam<String, false>,
) -> JsonResult<DestinationsListResponse> {
    let offset = from.into_inner().unwrap_or(0);
    let limit = limit.into_inner().unwrap_or(100);

    let servers = data::sending::get_all_destinations()?;
    let total = servers.len() as i64;

    let destinations: Vec<DestinationInfo> = servers
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|server| DestinationInfo {
            destination: server.to_string(),
            retry_last_ts: 0,
            retry_interval: 0,
            failure_ts: None,
            last_successful_stream_ordering: None,
        })
        .collect();

    let next_token = if (offset + destinations.len() as i64) < total {
        Some((offset + destinations.len() as i64).to_string())
    } else {
        None
    };

    json_ok(DestinationsListResponse {
        destinations,
        total,
        next_token,
    })
}

/// GET /_synapse/admin/v1/federation/destinations/{destination}
///
/// Get details of a specific destination
#[endpoint]
pub fn get_destination(destination: PathParam<OwnedServerName>) -> JsonResult<DestinationInfo> {
    let destination = destination.into_inner();

    // Check if destination is known
    if !data::sending::is_destination_known(&destination)? {
        return Err(MatrixError::not_found("Unknown destination").into());
    }

    json_ok(DestinationInfo {
        destination: destination.to_string(),
        retry_last_ts: 0,
        retry_interval: 0,
        failure_ts: None,
        last_successful_stream_ordering: None,
    })
}

/// GET /_synapse/admin/v1/federation/destinations/{destination}/rooms
///
/// Get rooms shared with a destination
#[endpoint]
pub fn destination_rooms(
    destination: PathParam<OwnedServerName>,
    from: QueryParam<i64, false>,
    limit: QueryParam<i64, false>,
) -> JsonResult<DestinationRoomsResponse> {
    let destination = destination.into_inner();
    let offset = from.into_inner().unwrap_or(0);
    let limit = limit.into_inner().unwrap_or(100);

    // Check if destination is known
    if !data::sending::is_destination_known(&destination)? {
        return Err(MatrixError::not_found("Unknown destination").into());
    }

    let rooms = data::sending::get_destination_rooms(&destination)?;
    let total = rooms.len() as i64;

    let rooms: Vec<DestinationRoom> = rooms
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|room_id| DestinationRoom {
            room_id: room_id.to_string(),
            stream_ordering: 0,
        })
        .collect();

    let next_token = if (offset + rooms.len() as i64) < total {
        Some((offset + rooms.len() as i64).to_string())
    } else {
        None
    };

    json_ok(DestinationRoomsResponse {
        rooms,
        total,
        next_token,
    })
}

/// POST /_synapse/admin/v1/federation/destinations/{destination}/reset_connection
///
/// Reset connection to a destination
#[endpoint]
pub fn reset_connection(destination: PathParam<OwnedServerName>) -> JsonResult<serde_json::Value> {
    let destination = destination.into_inner();

    // Check if destination is known
    if !data::sending::is_destination_known(&destination)? {
        return Err(MatrixError::not_found("Unknown destination").into());
    }

    // Reset retry timings
    data::sending::reset_destination_retry(&destination)?;

    json_ok(serde_json::json!({}))
}
