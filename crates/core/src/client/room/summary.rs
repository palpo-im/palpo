use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::EventEncryptionAlgorithm;
use crate::events::room::member::MembershipState;
use crate::identifiers::*;
use crate::room::RoomType;
use crate::space::SpaceRoomJoinRule;

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessTokenOptional,
//     history: {
//         unstable =>
// "/_matrix/client/unstable/im.nheko.summary/summary/:room_id_or_alias",
//         //1.15 => "/_matrix/client/v1/summary/:room_id_or_alias",
//     }
// };

/// Request type for the `get_summary` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct SummaryMsc3266ReqArgs {
    /// Alias or ID of the room to be summarized.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id_or_alias: OwnedRoomOrAliasId,

    /// Limit messages chunks size
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<OwnedServerName>,
}

/// Response type for the `get_room_event` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Debug)]
pub struct SummaryMsc3266ResBody {
    /// ID of the room (useful if it's an alias).
    pub room_id: OwnedRoomId,

    /// The canonical alias for this room, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_alias: Option<OwnedRoomAliasId>,

    /// Avatar of the room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<OwnedMxcUri>,

    /// Whether guests can join the room.
    pub guest_can_join: bool,

    /// Name of the room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Member count of the room.
    pub num_joined_members: u64,

    /// Topic of the room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,

    /// Whether the room history can be read without joining.
    pub world_readable: bool,

    /// Join rule of the room.
    pub join_rule: SpaceRoomJoinRule,

    /// Type of the room, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_type: Option<RoomType>,

    /// Version of the room.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "im.nheko.summary.room_version",
        alias = "im.nheko.summary.version",
        alias = "room_version"
    )]
    pub room_version: Option<RoomVersionId>,

    /// The current membership of this user in the room.
    ///
    /// This field will not be present when called unauthenticated, but is
    /// required when called authenticated. It should be `leave` if the
    /// server doesn't know about the room, since for all other membership
    /// states the server would know about the room already.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub membership: Option<MembershipState>,

    /// If the room is encrypted, the algorithm used for this room.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "im.nheko.summary.encryption",
        alias = "encryption"
    )]
    pub encryption: Option<EventEncryptionAlgorithm>,

    /// If the room is a restricted room, these are the room IDs which are
    /// specified by the join rules.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub allowed_room_ids: Vec<OwnedRoomId>,
}

impl SummaryMsc3266ResBody {
    /// Creates a new [`Response`] with all the mandatory fields set.
    pub fn new(
        room_id: OwnedRoomId,
        join_rule: SpaceRoomJoinRule,
        guest_can_join: bool,
        num_joined_members: u64,
        world_readable: bool,
    ) -> Self {
        Self {
            room_id,
            canonical_alias: None,
            avatar_url: None,
            guest_can_join,
            name: None,
            num_joined_members,
            topic: None,
            world_readable,
            join_rule,
            room_type: None,
            room_version: None,
            membership: None,
            encryption: None,
            allowed_room_ids: Vec::new(),
        }
    }
}
