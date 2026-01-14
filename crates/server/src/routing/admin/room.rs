use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::admin;
use crate::core::client::space::{HierarchyReqArgs, HierarchyResBody};
use crate::core::identifiers::*;
use crate::exts::IsRemoteOrLocal;
use crate::{AuthArgs, DepotExt, JsonResult, MatrixError, json_ok, room};

pub fn router() -> Router {
    Router::new()
        .push(
            Router::with_path("v1").push(
                Router::with_path("rooms")
                    .get(list_rooms)
                    .push(
                        Router::with_path("{room_id}")
                            .get(get_room)
                            .push(Router::with_path("hierarchy").get(get_hierarchy))
                            .push(Router::with_path("members").get(get_room_members))
                            .push(Router::with_path("state").get(get_room_state))
                            .push(Router::with_path("messages").get(get_room_messages))
                            .push(
                                Router::with_path("block")
                                    .get(get_room_block)
                                    .put(set_room_block),
                            )
                            .push(
                                Router::with_path("forward_extremities")
                                    .get(get_forward_extremities),
                            ),
                    ),
            ),
        )
        .push(
            Router::with_path("v2").push(
                Router::with_path("rooms/{room_id}")
                    .delete(delete_room),
            ),
        )
}

/// Room info response (detailed)
#[derive(Debug, Serialize, ToSchema)]
pub struct RoomInfoResponse {
    pub room_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_alias: Option<String>,
    pub joined_members: u64,
    pub joined_local_members: u64,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<String>,
    pub federatable: bool,
    pub public: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join_rules: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guest_access: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_visibility: Option<String>,
    pub state_events: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub forgotten: bool,
}

/// Room list response
#[derive(Debug, Serialize, ToSchema)]
pub struct RoomsListResponse {
    pub rooms: Vec<RoomInfoResponse>,
    pub offset: i64,
    pub total_rooms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_batch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_batch: Option<String>,
}

/// Room members response
#[derive(Debug, Serialize, ToSchema)]
pub struct RoomMembersResponse {
    pub members: Vec<String>,
    pub total: u64,
}

/// Room state response
#[derive(Debug, Serialize, ToSchema)]
pub struct RoomStateResponse {
    pub state: Vec<JsonValue>,
}

/// Room messages response
#[derive(Debug, Serialize, ToSchema)]
pub struct RoomMessagesResponse {
    pub chunk: Vec<JsonValue>,
    pub start: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

/// Room block status
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RoomBlockStatus {
    pub block: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

/// Delete room request
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteRoomReqBody {
    #[serde(default)]
    pub block: bool,
    #[serde(default = "default_purge")]
    pub purge: bool,
}

fn default_purge() -> bool {
    true
}

/// Delete room response
#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteRoomResponse {
    pub kicked_users: Vec<String>,
    pub failed_to_kick_users: Vec<String>,
    pub local_aliases: Vec<String>,
    pub new_room_id: Option<String>,
}

/// Forward extremities response
#[derive(Debug, Serialize, ToSchema)]
pub struct ForwardExtremitiesResponse {
    pub count: u64,
    pub results: Vec<ForwardExtremity>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ForwardExtremity {
    pub event_id: String,
    pub state_group: Option<i64>,
    pub depth: i64,
    pub received_ts: i64,
}

fn get_detailed_room_info(room_id: &RoomId) -> RoomInfoResponse {
    let basic_info = admin::get_room_info(room_id);

    let version = room::get_version(room_id)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let creator = room::get_create(room_id)
        .ok()
        .and_then(|c| c.creator().ok().map(|u| u.to_string()));

    let canonical_alias = room::get_canonical_alias(room_id)
        .ok()
        .flatten()
        .map(|a| a.to_string());

    let encryption = room::get_encryption(room_id)
        .ok()
        .map(|e| e.to_string());

    let join_rules = room::get_join_rule(room_id)
        .ok()
        .map(|r| format!("{:?}", r).to_lowercase());

    let history_visibility = room::get_history_visibility(room_id)
        .ok()
        .map(|h| format!("{:?}", h).to_lowercase());

    let topic = room::get_topic(room_id).ok();

    let avatar = room::get_avatar_url(room_id)
        .ok()
        .flatten()
        .map(|u| u.to_string());

    let room_type = room::get_room_type(room_id)
        .ok()
        .flatten()
        .map(|t| t.to_string());

    let joined_local_members = room::local_users_in_room(room_id)
        .map(|u| u.len() as u64)
        .unwrap_or(0);

    let state_events = room::get_current_frame_id(room_id)
        .ok()
        .flatten()
        .and_then(|frame_id| room::state::get_full_state_ids(frame_id).ok())
        .map(|s| s.len() as u64)
        .unwrap_or(0);

    let guest_access = room::guest_can_join(room_id);

    let is_public = crate::data::room::is_public(room_id).unwrap_or(false);

    RoomInfoResponse {
        room_id: room_id.to_string(),
        name: Some(basic_info.name).filter(|n| !n.is_empty()),
        canonical_alias,
        joined_members: basic_info.joined_members,
        joined_local_members,
        version,
        creator,
        encryption,
        federatable: true,
        public: is_public,
        join_rules,
        guest_access: Some(if guest_access { "can_join" } else { "forbidden" }.to_string()),
        history_visibility,
        state_events,
        room_type,
        topic,
        avatar,
        forgotten: false,
    }
}

/// List all rooms
#[endpoint]
pub fn list_rooms(
    from: QueryParam<i64, false>,
    limit: QueryParam<i64, false>,
    order_by: QueryParam<String, false>,
    dir: QueryParam<String, false>,
    search_term: QueryParam<String, false>,
) -> JsonResult<RoomsListResponse> {
    let offset = from.into_inner().unwrap_or(0).max(0);
    let limit = limit.into_inner().unwrap_or(100).clamp(1, 1000);
    let search_term = search_term.into_inner();
    let dir = dir.into_inner().unwrap_or_else(|| "f".to_string());
    let order_by_field = order_by.into_inner().unwrap_or_else(|| "name".to_string());

    let all_rooms = crate::room::all_room_ids()?;

    let mut rooms: Vec<RoomInfoResponse> = all_rooms
        .iter()
        .map(|room_id| get_detailed_room_info(room_id))
        .collect();

    // Filter by search term
    if let Some(ref term) = search_term {
        let term_lower = term.to_lowercase();
        rooms.retain(|r| {
            r.room_id.to_lowercase().contains(&term_lower)
                || r.name.as_ref().map_or(false, |n| n.to_lowercase().contains(&term_lower))
                || r.canonical_alias.as_ref().map_or(false, |a| a.to_lowercase().contains(&term_lower))
        });
    }

    // Sort rooms
    match order_by_field.as_str() {
        "joined_members" => rooms.sort_by_key(|r| r.joined_members),
        "joined_local_members" => rooms.sort_by_key(|r| r.joined_local_members),
        "version" => rooms.sort_by(|a, b| a.version.cmp(&b.version)),
        "creator" => rooms.sort_by(|a, b| a.creator.cmp(&b.creator)),
        "canonical_alias" => rooms.sort_by(|a, b| a.canonical_alias.cmp(&b.canonical_alias)),
        "state_events" => rooms.sort_by_key(|r| r.state_events),
        "room_type" => rooms.sort_by(|a, b| a.room_type.cmp(&b.room_type)),
        _ => rooms.sort_by(|a, b| a.name.cmp(&b.name)),
    }

    if dir == "b" {
        rooms.reverse();
    }

    let total_rooms = rooms.len() as i64;

    let rooms: Vec<RoomInfoResponse> = rooms
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();

    let next_batch = if (offset + rooms.len() as i64) < total_rooms {
        Some((offset + rooms.len() as i64).to_string())
    } else {
        None
    };

    let prev_batch = if offset > 0 {
        Some((offset - limit).max(0).to_string())
    } else {
        None
    };

    json_ok(RoomsListResponse {
        rooms,
        offset,
        total_rooms,
        next_batch,
        prev_batch,
    })
}

/// Get room details
#[endpoint]
pub fn get_room(room_id: PathParam<OwnedRoomId>) -> JsonResult<RoomInfoResponse> {
    let room_id = room_id.into_inner();

    if !room::room_exists(&room_id)? {
        return Err(MatrixError::not_found("Room not found").into());
    }

    json_ok(get_detailed_room_info(&room_id))
}

/// Get room hierarchy
#[endpoint]
pub async fn get_hierarchy(
    _aa: AuthArgs,
    args: HierarchyReqArgs,
    depot: &mut Depot,
) -> JsonResult<HierarchyResBody> {
    let authed = depot.authed_info()?;
    let res_body = crate::room::space::get_room_hierarchy(authed.user_id(), &args).await?;
    json_ok(res_body)
}

/// Get room members
#[endpoint]
pub fn get_room_members(room_id: PathParam<OwnedRoomId>) -> JsonResult<RoomMembersResponse> {
    let room_id = room_id.into_inner();

    if !room::room_exists(&room_id)? {
        return Err(MatrixError::not_found("Room not found").into());
    }

    let members = room::get_members(&room_id)?;
    let total = members.len() as u64;
    let members: Vec<String> = members.into_iter().map(|u| u.to_string()).collect();

    json_ok(RoomMembersResponse { members, total })
}

/// Get room state
#[endpoint]
pub fn get_room_state(room_id: PathParam<OwnedRoomId>) -> JsonResult<RoomStateResponse> {
    let room_id = room_id.into_inner();

    if !room::room_exists(&room_id)? {
        return Err(MatrixError::not_found("Room not found").into());
    }

    let frame_id = room::get_current_frame_id(&room_id)?
        .ok_or_else(|| MatrixError::not_found("Room has no state"))?;

    let state_map = room::state::get_full_state(frame_id)?;

    let state: Vec<JsonValue> = state_map
        .values()
        .filter_map(|pdu| serde_json::to_value(pdu).ok())
        .collect();

    json_ok(RoomStateResponse { state })
}

/// Get room messages
#[endpoint]
pub fn get_room_messages(
    room_id: PathParam<OwnedRoomId>,
    from: QueryParam<String, false>,
    limit: QueryParam<i64, false>,
    dir: QueryParam<String, false>,
) -> JsonResult<RoomMessagesResponse> {
    let room_id = room_id.into_inner();
    let from_token = from.into_inner();
    let limit = limit.into_inner().unwrap_or(10).clamp(1, 1000);
    let dir = dir.into_inner().unwrap_or_else(|| "b".to_string()); // Default backward like Synapse

    if !room::room_exists(&room_id)? {
        return Err(MatrixError::not_found("Room not found").into());
    }

    let backward = dir == "b";

    // Parse from_sn from token, or use appropriate default based on direction
    let from_sn: Option<i64> = from_token
        .as_ref()
        .and_then(|t| t.strip_prefix("sn_"))
        .and_then(|s| s.parse().ok());

    let db_events = crate::data::room::timeline::get_pdus_by_room(
        &room_id,
        from_sn,
        limit,
        backward,
    )?;

    // Get actual PDU content from event_datas
    let chunk: Vec<JsonValue> = db_events
        .iter()
        .filter_map(|e| {
            room::timeline::get_pdu(&e.id)
                .ok()
                .and_then(|pdu| serde_json::to_value(&pdu.pdu).ok())
        })
        .collect();

    let start = from_token.unwrap_or_else(|| {
        db_events
            .first()
            .map(|e| format!("sn_{}", e.sn))
            .unwrap_or_else(|| "sn_0".to_string())
    });
    let end = db_events.last().map(|e| format!("sn_{}", e.sn));

    json_ok(RoomMessagesResponse { chunk, start, end })
}

/// Get room block status
#[endpoint]
pub fn get_room_block(room_id: PathParam<OwnedRoomId>) -> JsonResult<RoomBlockStatus> {
    let room_id = room_id.into_inner();
    let blocked = crate::data::room::is_banned(&room_id).unwrap_or(false);

    json_ok(RoomBlockStatus {
        block: blocked,
        user_id: None,
    })
}

/// Set room block status
#[endpoint]
pub fn set_room_block(
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<RoomBlockStatus>,
) -> JsonResult<RoomBlockStatus> {
    let room_id = room_id.into_inner();
    let body = body.into_inner();

    room::ban_room(&room_id, body.block)?;

    json_ok(RoomBlockStatus {
        block: body.block,
        user_id: None,
    })
}

/// Delete room (v2)
///
/// Note: The `purge` field is currently not fully implemented.
/// When purge=true, events are not actually deleted from the database.
/// This is a known limitation - full purge requires additional data layer support.
#[endpoint]
pub async fn delete_room(
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<DeleteRoomReqBody>,
) -> JsonResult<DeleteRoomResponse> {
    let room_id = room_id.into_inner();
    let body = body.into_inner();

    if !room::room_exists(&room_id)? {
        return Err(MatrixError::not_found("Room not found").into());
    }

    // Explicitly reject purge requests until full purge is implemented
    if body.purge {
        return Err(MatrixError::bad_status(
            Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
            "Room purge is not supported yet",
        )
        .into());
    }

    let members = room::get_members(&room_id)?;
    let kicked_users: Vec<String> = members
        .iter()
        .filter(|m| m.server_name().is_local())
        .map(|m| m.to_string())
        .collect();

    let local_aliases: Vec<String> = room::local_aliases_for_room(&room_id)?
        .into_iter()
        .map(|a| a.to_string())
        .collect();

    if body.block {
        room::ban_room(&room_id, true)?;
    }

    // Disable the room (prevents new joins/messages)
    room::disable_room(&room_id, true)?;

    // TODO: Implement actual purge when purge=true
    // This would require:
    // 1. Delete events from events table
    // 2. Delete event data from event_datas table
    // 3. Delete state from room_state tables
    // 4. Clean up room_users entries
    // For now, we only disable the room

    json_ok(DeleteRoomResponse {
        kicked_users,
        failed_to_kick_users: Vec::new(),
        local_aliases,
        new_room_id: None,
    })
}

/// Get forward extremities
#[endpoint]
pub fn get_forward_extremities(
    room_id: PathParam<OwnedRoomId>,
) -> JsonResult<ForwardExtremitiesResponse> {
    let room_id = room_id.into_inner();

    if !room::room_exists(&room_id)? {
        return Err(MatrixError::not_found("Room not found").into());
    }

    let extremities = room::state::get_forward_extremities(&room_id)?;

    let results: Vec<ForwardExtremity> = extremities
        .into_iter()
        .filter_map(|event_id| {
            room::timeline::get_pdu(&event_id).ok().map(|pdu| {
                ForwardExtremity {
                    event_id: event_id.to_string(),
                    state_group: None,
                    depth: pdu.depth as i64,
                    received_ts: pdu.origin_server_ts.get() as i64,
                }
            })
        })
        .collect();

    json_ok(ForwardExtremitiesResponse {
        count: results.len() as u64,
        results,
    })
}
