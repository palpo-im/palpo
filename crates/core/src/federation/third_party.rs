/// Module for dealing with third party identifiers
///
/// `PUT /_matrix/federation/*/3pid/onbind`
///
/// Used by identity servers to notify the homeserver that one of its users has
/// bound a third party identifier successfully, including any pending room
/// invites the identity server has been made aware of.
/// `/v1/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/server-server-api/#put_matrixfederationv13pidonbind
use std::collections::BTreeMap;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::StateEventType;
use crate::third_party::Medium;
use crate::{OwnedRoomId, OwnedServerName, OwnedServerSigningKeyId, OwnedUserId};
// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/federation/v1/3pid/onbind",
//     }
// };

/// Request type for the `bind_callback` endpoint.

#[derive(ToSchema, Deserialize, Debug)]
pub struct BindCallbackReqBody {
    /// The type of third party identifier.
    ///
    /// Currently only `Medium::Email` is supported.
    pub medium: Medium,

    /// The third party identifier itself.
    ///
    /// For example: an email address.
    pub address: String,

    /// The user that is now bound to the third party identifier.
    pub mxid: OwnedUserId,

    /// A list of pending invites that the third party identifier has received.
    pub invites: Vec<ThirdPartyInvite>,
}

/// A pending invite the third party identifier has received.
#[derive(ToSchema, Debug, Clone, Deserialize, Serialize)]
pub struct ThirdPartyInvite {
    /// The type of third party invite issues.
    ///
    /// Currently only `Medium::Email` is used.
    pub medium: Medium,

    /// The third party identifier that received the invite.
    pub address: String,

    /// The now-bound user ID that received the invite.
    pub mxid: OwnedUserId,

    /// The room ID the invite is valid for.
    pub room_id: OwnedRoomId,

    /// The user ID that sent the invite.
    pub sender: OwnedUserId,

    /// Signature from the identity server using a long-term private key.
    pub signed: BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, String>>,
}

impl ThirdPartyInvite {
    /// Creates a new third party invite with the given parameters.
    pub fn new(
        address: String,
        mxid: OwnedUserId,
        room_id: OwnedRoomId,
        sender: OwnedUserId,
        signed: BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, String>>,
    ) -> Self {
        Self {
            medium: Medium::Email,
            address,
            mxid,
            room_id,
            sender,
            signed,
        }
    }
}

// /// `PUT /_matrix/federation/*/exchange_third_party_invite/{room_id}`
// ///
// /// The receiving server will verify the partial `m.room.member` event given in
// /// the request body. If valid, the receiving server will issue an invite as per
// /// the [Inviting to a room] section before returning a response to this
// /// request.
// ///
// /// [Inviting to a room]: https://spec.matrix.org/latest/server-server-api/#inviting-to-a-room
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#put_matrixfederationv1exchange_third_party_inviteroomid
// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/federation/v1/exchange_third_party_invite/:room_id",
//     }
// };

/// Request type for the `exchange_invite` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct ExchangeInviteReqBody {
    /// The room ID to exchange a third party invite in.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The event type.
    ///
    /// Must be `StateEventType::RoomMember`.
    #[serde(rename = "type")]
    pub kind: StateEventType,

    /// The user ID of the user who sent the original invite event.
    pub sender: OwnedUserId,

    /// The user ID of the invited user.
    pub state_key: OwnedUserId,

    /// The content of the invite event.
    pub content: ThirdPartyInvite,
}
impl ExchangeInviteReqBody {
    /// Creates a new `Request` for a third party invite exchange
    pub fn new(
        room_id: OwnedRoomId,
        sender: OwnedUserId,
        state_key: OwnedUserId,
        content: ThirdPartyInvite,
    ) -> Self {
        Self {
            room_id,
            kind: StateEventType::RoomMember,
            sender,
            state_key,
            content,
        }
    }
}
