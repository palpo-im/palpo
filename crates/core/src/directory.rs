/// Common types for room directory endpoints.
use serde::{Deserialize, Serialize};
mod filter_room_type_serde;
mod room_network_serde;
use salvo::prelude::*;

use crate::room::RoomType;
use crate::serde::StringEnum;
use crate::{OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId, PrivOwnedStr, UnixMillis};

/// A chunk of a room list response, describing one room.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct PublicRoomsChunk {
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
    #[serde(default)]
    pub join_rule: PublicRoomJoinRule,

    /// The type of room from `m.room.create`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room_type: Option<RoomType>,
}

/// A filter for public rooms lists.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct PublicRoomFilter {
    /// A string to search for in the room e.g. name, topic, canonical alias
    /// etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generic_search_term: Option<String>,

    /// The room types to include in the results.
    ///
    /// Includes all room types if it is empty.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::serde::none_as_default"
    )]
    pub room_types: Vec<RoomTypeFilter>,
}

impl PublicRoomFilter {
    /// Creates an empty `Filter`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns `true` if the filter is empty.
    pub fn is_empty(&self) -> bool {
        self.generic_search_term.is_none()
    }
}

/// Information about which networks/protocols from application services on the
/// homeserver from which to request rooms.
#[derive(ToSchema, Clone, Debug, Default, PartialEq, Eq)]
pub enum RoomNetwork {
    /// Return rooms from the Matrix network.
    #[default]
    Matrix,

    /// Return rooms from all the networks/protocols the homeserver knows about.
    All,

    /// Return rooms from a specific third party network/protocol.
    ThirdParty(String),
}

/// The rule used for users wishing to join a public room.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, Default, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
pub enum PublicRoomJoinRule {
    /// Users can request an invite to the room.
    Knock,

    KnockRestricted,

    /// Anyone can join the room without any prior action.
    #[default]
    Public,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

/// An enum of possible room types to filter.
///
/// This type can hold an arbitrary string. To build this with a custom value,
/// convert it from an `Option<string>` with `::from()` / `.into()`.
/// [`RoomTypeFilter::Default`] can be constructed from `None`.
///
/// To check for values that are not available as a documented variant here, use
/// its string representation, obtained through [`.as_str()`](Self::as_str()).
#[derive(ToSchema, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RoomTypeFilter {
    /// The default room type, defined without a `room_type`.
    Default,

    /// A space.
    Space,

    /// A custom room type.
    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

impl RoomTypeFilter {
    /// Get the string representation of this `RoomTypeFilter`.
    ///
    /// [`RoomTypeFilter::Default`] returns `None`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            RoomTypeFilter::Default => None,
            RoomTypeFilter::Space => Some("m.space"),
            RoomTypeFilter::_Custom(s) => Some(&s.0),
        }
    }
}

impl<T> From<Option<T>> for RoomTypeFilter
where
    T: AsRef<str> + Into<Box<str>>,
{
    fn from(s: Option<T>) -> Self {
        match s {
            None => Self::Default,
            Some(s) => match s.as_ref() {
                "m.space" => Self::Space,
                _ => Self::_Custom(PrivOwnedStr(s.into())),
            },
        }
    }
}

impl From<Option<RoomType>> for RoomTypeFilter {
    fn from(t: Option<RoomType>) -> Self {
        match t {
            None => Self::Default,
            Some(s) => match s {
                RoomType::Space => Self::Space,
                _ => Self::from(Some(s.as_str())),
            },
        }
    }
}

/// The query criteria.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct QueryCriteria {
    /// A millisecond POSIX timestamp in milliseconds indicating when the
    /// returned certificates will need to be valid until to be useful to the
    /// requesting server.
    ///
    /// If not supplied, the current time as determined by the notary server is
    /// used.
    // This doesn't use `serde(default)` because the default would then be
    // determined by the client rather than the server (and it would take more
    // bandwidth because `skip_serializing_if` couldn't be used).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_valid_until_ts: Option<UnixMillis>,
}

impl QueryCriteria {
    /// Creates empty `QueryCriteria`.
    pub fn new() -> Self {
        Default::default()
    }
}

/// Arbitrary values that identify this implementation.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct Server {
    /// Arbitrary name that identifies this implementation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Version of this implementation.
    ///
    /// The version format depends on the implementation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl Server {
    /// Creates an empty `Server`.
    pub fn new() -> Self {
        Default::default()
    }
}

// /// Request type for the `get_public_rooms` endpoint.

/// Response type for the `get_public_rooms` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Default, Debug)]
pub struct PublicRoomsResBody {
    /// A paginated chunk of public rooms.
    pub chunk: Vec<PublicRoomsChunk>,

    /// A pagination token for the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_batch: Option<String>,

    /// A pagination token that allows fetching previous results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_batch: Option<String>,

    /// An estimate on the total number of public rooms, if the server has an
    /// estimate.
    pub total_room_count_estimate: Option<u64>,
}

impl PublicRoomsResBody {
    /// Creates an empty `Response`.
    pub fn new() -> Self {
        Default::default()
    }
}
