use itertools::Itertools;
use reqwest::Url;
/// Endpoints for handling room knocking.
/// `GET /_matrix/federation/*/make_knock/{room_id}/{user_id}`
///
/// Send a request for a knock event template to a resident server.
/// `/v1/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1make_knockroomiduser_id
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::AnyStrippedStateEvent;
use crate::sending::{SendRequest, SendResult};
use crate::serde::{RawJson, RawJsonValue};
use crate::{OwnedEventId, OwnedRoomId, OwnedUserId, RoomVersionId};
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         unstable =>
// "/_matrix/federation/unstable/xyz.amorgan.knock/make_knock/:room_id/:user_id"
// ,         1.1 => "/_matrix/federation/v1/make_knock/:room_id/:user_id",
//     }
// };

pub fn make_knock_request(origin: &str, args: MakeKnockReqArgs) -> SendResult<SendRequest> {
    let ver = args.ver.iter().map(|v| format!("ver={v}")).join("&");
    let ver = if ver.is_empty() {
        ""
    } else {
        &*format!("?{ver}")
    };
    let url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/make_knock/{}/{}{ver}",
        args.room_id, args.user_id
    ))?;
    Ok(crate::sending::get(url))
}

/// Request type for the `create_knock_event_template` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct MakeKnockReqArgs {
    /// The room ID that should receive the knock.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The user ID the knock event will be for.
    #[salvo(parameter(parameter_in = Path))]
    pub user_id: OwnedUserId,

    /// The room versions the sending has support for.
    ///
    /// Defaults to `vec![RoomVersionId::V1]`.
    #[salvo(parameter(parameter_in = Query))]
    pub ver: Vec<RoomVersionId>,
}

/// Response type for the `create_knock_event_template` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]

pub struct MakeKnockResBody {
    /// The version of the room where the server is trying to knock.
    pub room_version: RoomVersionId,

    /// An unsigned template event.
    ///
    /// May differ between room versions.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub event: Box<RawJsonValue>,
}

impl MakeKnockResBody {
    /// Creates a new `Response` with the given room version ID and event.
    pub fn new(room_version: RoomVersionId, event: Box<RawJsonValue>) -> Self {
        Self {
            room_version,
            event,
        }
    }
}

// /// `PUT /_matrix/federation/*/send_knock/{room_id}/{event_id}`
// ///
// /// Submits a signed knock event to the resident homeserver for it to accept
// /// into the room's graph. `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#put_matrixfederationv1send_knockroomideventid

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         unstable =>
// "/_matrix/federation/unstable/xyz.amorgan.knock/send_knock/:room_id/:
// event_id",         1.1 =>
// "/_matrix/federation/v1/send_knock/:room_id/:event_id",     }
// };

pub fn send_knock_request(
    origin: &str,
    args: SendKnockReqArgs,
    body: SendKnockReqBody,
) -> SendResult<SendRequest> {
    let url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/send_knock/{}/{}",
        args.room_id, args.event_id
    ))?;
    crate::sending::put(url).stuff(body)
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct SendKnockReqArgs {
    /// The room ID that should receive the knock.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The user ID the knock event will be for.
    #[salvo(parameter(parameter_in = Path))]
    pub event_id: OwnedEventId,
}

/// Request type for the `send_knock` endpoint.

#[derive(ToSchema, Serialize, Deserialize, Debug)]
#[salvo(schema(value_type = Object))]
pub struct SendKnockReqBody(pub Box<RawJsonValue>);

impl SendKnockReqBody {
    /// Creates a new `Request` with the given PDU.
    pub fn new(pdu: Box<RawJsonValue>) -> Self {
        Self(pdu)
    }
}
crate::json_body_modifier!(SendKnockReqBody);

/// Response type for the `send_knock` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]

pub struct SendKnockResBody {
    /// State events providing public room metadata.
    pub knock_room_state: Vec<RawJson<AnyStrippedStateEvent>>,
}

impl SendKnockResBody {
    /// Creates a new `Response` with the given public room metadata state
    /// events.
    pub fn new(knock_room_state: Vec<RawJson<AnyStrippedStateEvent>>) -> Self {
        Self { knock_room_state }
    }
}
