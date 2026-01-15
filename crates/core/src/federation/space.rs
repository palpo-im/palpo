use reqwest::Url;
use salvo::prelude::*;
/// Spaces endpoints.
use serde::{Deserialize, Serialize};

use crate::events::space::child::HierarchySpaceChildEvent;
use crate::room::RoomType;
use crate::sending::{SendRequest, SendResult};
use crate::serde::RawJson;
use crate::space::SpaceRoomJoinRule;
use crate::{EventEncryptionAlgorithm, OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId, RoomVersionId};

/// The summary of a parent space.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct SpaceHierarchyParentSummary {
    /// The canonical alias of the room, if any.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "crate::serde::empty_string_as_none"
    )]
    pub canonical_alias: Option<OwnedRoomAliasId>,

    /// The name of the room, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The number of members joined to the room.
    pub num_joined_members: u64,

    /// The ID of the room.
    pub room_id: OwnedRoomId,

    /// The topic of the room, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,

    /// Whether the room may be viewed by guest users without joining.
    pub world_readable: bool,

    /// Whether guest users may join the room and participate in it.
    ///
    /// If they can, they will be subject to ordinary power level rules like any
    /// other user.
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

    /// If the room is a restricted room, these are the room IDs which are
    /// specified by the join rules.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub allowed_room_ids: Vec<OwnedRoomId>,

    /// If the room is encrypted, the algorithm used for this room.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "im.nheko.summary.encryption",
        alias = "encryption"
    )]
    pub encryption: Option<EventEncryptionAlgorithm>,

    /// Version of the room.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "im.nheko.summary.room_version",
        alias = "im.nheko.summary.version",
        alias = "room_version"
    )]
    pub room_version: Option<RoomVersionId>,
}

/// The summary of a space's child.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct SpaceHierarchyChildSummary {
    /// The canonical alias of the room, if any.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "crate::serde::empty_string_as_none"
    )]
    pub canonical_alias: Option<OwnedRoomAliasId>,

    /// The name of the room, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The number of members joined to the room.
    pub num_joined_members: u64,

    /// The ID of the room.
    pub room_id: OwnedRoomId,

    /// The topic of the room, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,

    /// Whether the room may be viewed by guest users without joining.
    pub world_readable: bool,

    /// Whether guest users may join the room and participate in it.
    ///
    /// If they can, they will be subject to ordinary power level rules like any
    /// other user.
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

    /// If the room is a restricted room, these are the room IDs which are
    /// specified by the join rules.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub allowed_room_ids: Vec<OwnedRoomId>,

    /// If the room is encrypted, the algorithm used for this room.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "im.nheko.summary.encryption",
        alias = "encryption"
    )]
    pub encryption: Option<EventEncryptionAlgorithm>,

    /// Version of the room.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "im.nheko.summary.room_version",
        alias = "im.nheko.summary.version",
        alias = "room_version"
    )]
    pub room_version: Option<RoomVersionId>,
}

impl From<SpaceHierarchyParentSummary> for SpaceHierarchyChildSummary {
    fn from(parent: SpaceHierarchyParentSummary) -> Self {
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
            children_state: _,
            allowed_room_ids,
            encryption,
            room_version,
        } = parent;

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
            allowed_room_ids,
            encryption,
            room_version,
        }
    }
}

// /// `GET /_matrix/federation/*/hierarchy/{room_id}`
// ///
// /// Get the space tree in a depth-first manner to locate child rooms of a given
// /// space. `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1hierarchyroomid
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         unstable =>
// "/_matrix/federation/unstable/org.matrix.msc2946/hierarchy/:room_id",
//         1.2 => "/_matrix/federation/v1/hierarchy/:room_id",
//     }
// };

pub fn hierarchy_request(origin: &str, args: HierarchyReqArgs) -> SendResult<SendRequest> {
    let url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/hierarchy/{}?suggested_only={}",
        args.room_id, args.suggested_only
    ))?;
    Ok(crate::sending::get(url))
}
/// Request type for the `hierarchy` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct HierarchyReqArgs {
    /// The room ID of the space to get a hierarchy for.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// Whether or not the server should only consider suggested rooms.
    ///
    /// Suggested rooms are annotated in their `m.space.child` event contents.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub suggested_only: bool,
}

/// Response type for the `hierarchy` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct HierarchyResBody {
    /// A summary of the space’s children.
    ///
    /// Rooms which the requesting server cannot peek/join will be excluded.
    pub children: Vec<SpaceHierarchyChildSummary>,

    /// The list of room IDs the requesting server doesn’t have a viable way to
    /// peek/join.
    ///
    /// Rooms which the responding server cannot provide details on will be
    /// outright excluded from the response instead.
    pub inaccessible_children: Vec<OwnedRoomId>,

    /// A summary of the requested room.
    pub room: SpaceHierarchyParentSummary,
}

impl HierarchyResBody {
    /// Creates a new `Response` with the given room summary.
    pub fn new(room_summary: SpaceHierarchyParentSummary) -> Self {
        Self {
            children: Vec::new(),
            inaccessible_children: Vec::new(),
            room: room_summary,
        }
    }
}
