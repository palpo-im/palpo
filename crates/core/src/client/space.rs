use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::space::child::HierarchySpaceChildEvent;
use crate::{OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId, room::RoomType, serde::RawJson, space::SpaceRoomJoinRule};

/// Endpoints for spaces.
///
/// See the [Matrix specification][spec] for more details about spaces.
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#spaces
/// A chunk of a space hierarchy response, describing one room.
///
/// To create an instance of this type, first create a `SpaceHierarchyRoomsChunkInit` and convert it
/// via `SpaceHierarchyRoomsChunk::from` / `.into()`.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct SpaceHierarchyRoomsChunk {
    /// The canonical alias of the room, if any.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "crate::serde::empty_string_as_none"
    )]
    pub canonical_alias: Option<OwnedRoomAliasId>,

    /// The name of the room, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The number of members joined to the room.
    #[serde(default)]
    pub num_joined_members: u64,

    /// The ID of the room.
    pub room_id: OwnedRoomId,

    /// The topic of the room, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,

    /// Whether the room may be viewed by guest users without joining.
    #[serde(default)]
    pub world_readable: bool,

    /// Whether guest users may join the room and participate in it.
    ///
    /// If they can, they will be subject to ordinary power level rules like any other user.
    #[serde(default)]
    pub guest_can_join: bool,

    /// The URL for the room's avatar, if one is set.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "crate::serde::empty_string_as_none"
    )]
    pub avatar_url: Option<OwnedMxcUri>,

    /// The join rule of the room.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub join_rule: SpaceRoomJoinRule,

    /// The type of room from `m.room.create`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_type: Option<RoomType>,

    /// The stripped `m.space.child` events of the space-room.
    ///
    /// If the room is not a space-room, this should be empty.
    pub children_state: Vec<RawJson<HierarchySpaceChildEvent>>,
}

/// Initial set of mandatory fields of `SpaceHierarchyRoomsChunk`.
///
/// This struct will not be updated even if additional fields are added to
/// `SpaceHierarchyRoomsChunk` in a new (non-breaking) release of the Matrix specification.
#[derive(Debug)]
#[allow(clippy::exhaustive_structs)]
pub struct SpaceHierarchyRoomsChunkInit {
    /// The number of members joined to the room.
    pub num_joined_members: u64,

    /// The ID of the room.
    pub room_id: OwnedRoomId,

    /// Whether the room may be viewed by guest users without joining.
    pub world_readable: bool,

    /// Whether guest users may join the room and participate in it.
    ///
    /// If they can, they will be subject to ordinary power level rules like any other user.
    pub guest_can_join: bool,

    /// The join rule of the room.
    pub join_rule: SpaceRoomJoinRule,

    /// The stripped `m.space.child` events of the space-room.
    ///
    /// If the room is not a space-room, this should be empty.
    pub children_state: Vec<RawJson<HierarchySpaceChildEvent>>,
}

impl From<SpaceHierarchyRoomsChunkInit> for SpaceHierarchyRoomsChunk {
    fn from(init: SpaceHierarchyRoomsChunkInit) -> Self {
        let SpaceHierarchyRoomsChunkInit {
            num_joined_members,
            room_id,
            world_readable,
            guest_can_join,
            join_rule,
            children_state,
        } = init;

        Self {
            canonical_alias: None,
            name: None,
            num_joined_members,
            room_id,
            topic: None,
            world_readable,
            guest_can_join,
            avatar_url: None,
            join_rule,
            room_type: None,
            children_state,
        }
    }
}

/// `GET /_matrix/client/*/rooms/{room_id}/hierarchy`
///
/// Paginates over the space tree in a depth-first manner to locate child rooms of a given space.
/// `/v1/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv1roomsroomidhierarchy
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/org.matrix.msc2946/rooms/:room_id/hierarchy",
//         1.2 => "/_matrix/client/v1/rooms/:room_id/hierarchy",
//     }
// };

/// Request type for the `hierarchy` endpoint.    
#[derive(ToParameters, Deserialize, Debug)]
pub struct HierarchyReqArgs {
    /// The room ID of the space to get a hierarchy for.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// A pagination token from a previous result.
    ///
    /// If specified, `max_depth` and `suggested_only` cannot be changed from the first
    /// request.
    #[salvo(parameter(parameter_in = Query))]
    pub from: Option<String>,

    /// The maximum number of rooms to include per response.
    #[salvo(parameter(parameter_in = Query))]
    pub limit: Option<usize>,

    /// How far to go into the space.
    ///
    /// When reached, no further child rooms will be returned.
    #[salvo(parameter(parameter_in = Query))]
    pub max_depth: Option<usize>,

    /// Whether or not the server should only consider suggested rooms.
    ///
    /// Suggested rooms are annotated in their `m.space.child` event contents.
    ///
    /// Defaults to `false`.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub suggested_only: bool,
}

/// Response type for the `hierarchy` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct HierarchyResBody {
    /// A token to supply to from to keep paginating the responses.
    ///
    /// Not present when there are no further results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_batch: Option<String>,

    /// A paginated chunk of the space children.
    pub rooms: Vec<SpaceHierarchyRoomsChunk>,
}
