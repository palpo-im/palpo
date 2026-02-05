//! Endpoints for room membership.
use std::collections::BTreeMap;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::room::member::RoomMemberEvent;
use crate::serde::{JsonValue, RawJson, StringEnum};
use crate::third_party::Medium;
use crate::{
    OwnedMxcUri, OwnedRoomId, OwnedServerName, OwnedServerSigningKeyId, OwnedUserId, PrivOwnedStr,
};

/// A signature of an `m.third_party_invite` token to prove that this user owns
/// a third party identity which has been invited to the room.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct ThirdPartySigned {
    /// The Matrix ID of the user who issued the invite.
    pub sender: OwnedUserId,

    /// The Matrix ID of the invitee.
    pub mxid: OwnedUserId,

    /// The state key of the `m.third_party_invite` event.
    pub token: String,

    /// A signatures object containing a signature of the entire signed object.
    pub signatures: BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, String>>,
}

impl ThirdPartySigned {
    /// Creates a new `ThirdPartySigned` from the given sender and invitee user
    /// IDs, state key token and signatures.
    pub fn new(
        sender: OwnedUserId,
        mxid: OwnedUserId,
        token: String,
        signatures: BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, String>>,
    ) -> Self {
        Self {
            sender,
            mxid,
            token,
            signatures,
        }
    }
}

/// Represents third party IDs to invite to the room.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct InviteThreepid {
    /// Hostname and port of identity server to be used for account lookups.
    pub id_server: String,

    /// An access token registered with the identity server.
    pub id_access_token: String,

    /// Type of third party ID.
    pub medium: Medium,

    /// Third party identifier.
    pub address: String,
}

// const METADATA: Metadata = metadata! {
//     method: `POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/ban",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/ban",
//     }
// };

/// Request type for the `ban_user` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct BanUserReqBody {
    // /// The room to kick the user from.
    // #[salvo(parameter(parameter_in = Path))]
    // pub room_id: OwnedRoomId,
    /// The user to ban.
    pub user_id: OwnedUserId,

    /// The reason for banning the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// /// `POST /_matrix/client/*/rooms/{room_id}/unban`
// ///
// /// Unban a user from a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3roomsroomidunban

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/unban",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/unban",
//     }
// };

/// Request type for the `unban_user` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct UnbanUserReqBody {
    // /// The room to unban the user from.
    // #[salvo(parameter(parameter_in = Path))]
    // pub room_id: OwnedRoomId,
    /// The user to unban.
    pub user_id: OwnedUserId,

    /// Optional reason for unbanning the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Distinguishes between invititations by Matrix or third party identifiers.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum InvitationRecipient {
    /// Used to invite user by their Matrix identifier.
    UserId {
        /// Matrix identifier of user.
        user_id: OwnedUserId,
    },

    /// Used to invite user by a third party identifier.
    ThirdPartyId(InviteThreepid),
}
// /// `POST /_matrix/client/*/rooms/{room_id}/kick`
// ///
// /// Kick a user from a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3roomsroomidkick

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/kick",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/kick",
//     }
// };

/// Request type for the `kick_user` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct KickUserReqBody {
    // /// The room to kick the user from.
    // #[salvo(parameter(parameter_in = Path))]
    // pub room_id: OwnedRoomId,
    /// The user to kick.
    pub user_id: OwnedUserId,

    /// The reason for kicking the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// /// `POST /_matrix/client/*/rooms/{room_id}/invite`
// ///
// /// Invite a user to a room.
// /// `/v3/` ([spec (MXID)][spec-mxid], [spec (3PID)][spec-3pid])
// ///
// /// This endpoint has two forms: one to invite a user
// /// [by their Matrix identifier][spec-mxid], and one to invite a user
// /// [by their third party identifier][spec-3pid].
// ///
// /// [spec-mxid]: https://spec.matrix.org/v1.9/client-server-api/#post_matrixclientv3roomsroomidinvite
// /// [spec-3pid]: https://spec.matrix.org/v1.9/client-server-api/#post_matrixclientv3roomsroomidinvite-1

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/invite",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/invite",
//     }
// };

/// Request type for the `invite_user` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct InviteUserReqBody {
    /// The user to invite.
    #[serde(flatten)]
    pub recipient: InvitationRecipient,

    /// Optional reason for inviting the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// /// `POST /_matrix/client/*/rooms/{room_id}/leave`
// ///
// /// Leave a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3roomsroomidleave
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/leave",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/leave",
//     }
// };

/// Request type for the `leave_room` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct LeaveRoomReqBody {
    /// Optional reason to be included as the `reason` on the subsequent
    /// membership event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// /// `GET /_matrix/client/*/user/mutual_rooms/{user_id}`
// ///
// /// Get mutual rooms with another user.
// /// `/unstable/` ([spec])
// ///
// /// [spec]: https://github.com/matrix-org/matrix-spec-proposals/blob/hs/shared-rooms/proposals/2666-get-rooms-in-common.md

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/uk.half-shot.msc2666/user/mutual_rooms",
//     }
// };

/// Request type for the `mutual_rooms` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct MutualRoomsReqArgs {
    /// The user to search mutual rooms for.
    #[salvo(parameter(parameter_in = Query))]
    pub user_id: OwnedUserId,

    /// The `next_batch_token` returned from a previous response, to get the
    /// next batch of rooms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub batch_token: Option<String>,
}

/// Response type for the `mutual_rooms` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct MutualRoomsResBody {
    /// A list of rooms the user is in together with the authenticated user.
    pub joined: Vec<OwnedRoomId>,

    /// An opaque string, returned when the server paginates this response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_batch_token: Option<String>,
}

impl MutualRoomsResBody {
    /// Creates a `Response` with the given room ids.
    pub fn new(joined: Vec<OwnedRoomId>) -> Self {
        Self {
            joined,
            next_batch_token: None,
        }
    }

    /// Creates a `Response` with the given room ids, together with a batch
    /// token.
    pub fn with_token(joined: Vec<OwnedRoomId>, token: String) -> Self {
        Self {
            joined,
            next_batch_token: Some(token),
        }
    }
}
// /// `GET /_matrix/client/*/joined_rooms`
// ///
// /// Get a list of the user's current rooms.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3joined_rooms

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/joined_rooms",
//         1.1 => "/_matrix/client/v3/joined_rooms",
//     }
// };

/// Response type for the `joined_rooms` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct JoinedRoomsResBody {
    /// A list of the rooms the user is in, i.e. the ID of each room in
    /// which the user has joined membership.
    pub joined_rooms: Vec<OwnedRoomId>,
}

impl JoinedRoomsResBody {
    /// Creates a new `Response` with the given joined rooms.
    pub fn new(joined_rooms: Vec<OwnedRoomId>) -> Self {
        Self { joined_rooms }
    }
}

// /// `POST /_matrix/client/*/rooms/{room_id}/forget`
// ///
// /// Forget a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3roomsroomidforget
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/forget",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/forget",
//     }
// };

// /// Request type for the `forget_room` endpoint.

// pub struct ForgetReqBody {
//     /// The room to forget.
//     #[salvo(parameter(parameter_in = Path))]
//     pub room_id: OwnedRoomId,
// }

// /// `POST /_matrix/client/*/rooms/{room_id}/join`
// ///
// /// Join a room using its ID.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3roomsroomidjoin
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/join",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/join",
//     }
// };

/// Request type for the `join_room_by_id` endpoint.
#[derive(ToSchema, Deserialize, Default, Debug)]
pub struct JoinRoomReqBody {
    /// The signature of a `m.third_party_invite` token to prove that this user
    /// owns a third party identity which has been invited to the room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub third_party_signed: Option<ThirdPartySigned>,

    /// Optional reason for joining the room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Optional extra parameters to be sent to the server.
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub extra_data: BTreeMap<String, JsonValue>,
}

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/join/:room_id_or_alias",
//         1.1 => "/_matrix/client/v3/join/:room_id_or_alias",
//     }
// };

// /// Request type for the `join_room_by_id_or_alias` endpoint.
// #[derive(ToSchema, Default, Deserialize, Debug)]
// pub struct JoinRoomByIdOrAliasReqBody {
//     /// The signature of a `m.third_party_invite` token to prove that this user
//     /// owns a third party identity which has been invited to the room.
//     pub third_party_signed: Option<ThirdPartySigned>,

//     /// Optional reason for joining the room.
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub reason: Option<String>,
// }

/// Response type for the `join_room_by_id` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct JoinRoomResBody {
    /// The room that the user joined.
    pub room_id: OwnedRoomId,
}
impl JoinRoomResBody {
    /// Creates a new `Response` with the given room id.
    pub fn new(room_id: OwnedRoomId) -> Self {
        Self { room_id }
    }
}

// /// `GET /_matrix/client/*/rooms/{room_id}/members`
// ///
// /// Get membership events for a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3roomsroomidmembers
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/members",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/members",
//     }
// };

/// Request type for the `get_member_events` endpoint.

#[derive(ToParameters, Deserialize, Debug)]
pub struct MembersReqArgs {
    /// The room to get the member events for.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The point in time (pagination token) to return members for in the room.
    ///
    /// This token can be obtained from a prev_batch token returned for each
    /// room by the sync API.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub at: Option<String>,

    /// The kind of memberships to filter for.
    ///
    /// Defaults to no filtering if unspecified. When specified alongside
    /// not_membership, the two parameters create an 'or' condition: either
    /// the membership is the same as membership or is not the same as
    /// not_membership.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub membership: Option<MembershipEventFilter>,

    /// The kind of memberships to *exclude* from the results.
    ///
    /// Defaults to no filtering if unspecified.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub not_membership: Option<MembershipEventFilter>,
}

/// Response type for the `get_member_events` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct MembersResBody {
    /// A list of member events.
    #[salvo(schema(value_type = Vec<Object>))]
    pub chunk: Vec<RawJson<RoomMemberEvent>>,
}
impl MembersResBody {
    /// Creates a new `Response` with the given member event chunk.
    pub fn new(chunk: Vec<RawJson<RoomMemberEvent>>) -> Self {
        Self { chunk }
    }
}

/// The kind of membership events to filter for.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "lowercase")]
#[non_exhaustive]
pub enum MembershipEventFilter {
    /// The user has joined.
    Join,

    /// The user has been invited.
    Invite,

    /// The user has left.
    Leave,

    /// The user has been banned.
    Ban,

    /// The user has knocked.
    Knock,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

// /// `GET /_matrix/client/*/rooms/{room_id}/joined_members`
// ///
// /// Get a map of user IDs to member info objects for members of the room.
// /// Primarily for use in Application Services.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3roomsroomidjoined_members
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/joined_members",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/joined_members",
//     }
// };

// /// Request type for the `joined_members` endpoint.

// pub struct JoinedMembersReqBody {
//     /// The room to get the members of.
//     #[salvo(parameter(parameter_in = Path))]
//     pub room_id: OwnedRoomId,
// }

/// Response type for the `joined_members` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct JoinedMembersResBody {
    /// A list of the rooms the user is in, i.e.
    /// the ID of each room in which the user has joined membership.
    pub joined: BTreeMap<OwnedUserId, RoomMember>,
}
impl JoinedMembersResBody {
    /// Creates a new `Response` with the given joined rooms.
    pub fn new(joined: BTreeMap<OwnedUserId, RoomMember>) -> Self {
        Self { joined }
    }
}

/// Information about a room member.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct RoomMember {
    /// The display name of the user.
    // #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub display_name: Option<String>,

    /// The mxc avatar url of the user.
    #[serde(
        // skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "crate::serde::empty_string_as_none"
    )]
    pub avatar_url: Option<OwnedMxcUri>,
}

impl RoomMember {
    /// Creates an empty `RoomMember`.
    pub fn new(display_name: Option<String>, avatar_url: Option<OwnedMxcUri>) -> Self {
        Self {
            display_name,
            avatar_url,
        }
    }
}
