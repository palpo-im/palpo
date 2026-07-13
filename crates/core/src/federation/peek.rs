//! `GET /_matrix/federation/v1/peek/{room_id}`
//!
//! Fetch a snapshot of a (world-readable) room over federation so a peeking
//! homeserver can render a preview for a user who has not joined it.
//!
//! This is a lightweight, Palpo-specific form of federated peeking (in the
//! spirit of MSC2444). Unlike a full peek subscription, it does not register
//! the peeking server for ongoing event distribution; it returns a one-shot
//! snapshot of the current resolved room state, the auth chain backing it, and
//! a page of the most recent timeline events — exactly what a client needs to
//! render an "untyped room preview". To follow new events the user joins the
//! room (which is a normal federated join).

use reqwest::Url;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::identifiers::*;
use crate::sending::{SendRequest, SendResult};
use crate::serde::RawJsonValue;

/// Build the outgoing request for a federated room peek.
pub fn peek_request(origin: &str, args: PeekReqArgs) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/peek/{}",
        args.room_id
    ))?;
    url.query_pairs_mut()
        .append_pair("limit", &args.limit.to_string());
    Ok(crate::sending::get(url))
}

/// Request type for the federated `peek` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct PeekReqArgs {
    /// The room to peek.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// Maximum number of recent timeline messages to return.
    #[salvo(parameter(parameter_in = Query))]
    pub limit: usize,
}

/// Response type for the federated `peek` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct PeekResBody {
    /// The version of the room being peeked.
    pub room_version: RoomVersionId,

    /// The room's public "description" state, limited to the recommended
    /// stripped-state event types (create, name, topic, avatar, join rules,
    /// canonical alias, encryption, history visibility). Membership, power
    /// levels and arbitrary custom state are deliberately excluded so a peek
    /// never exposes more than the spec's stripped-state preview surface.
    #[salvo(schema(value_type = Vec<Object>))]
    pub pdus: Vec<Box<RawJsonValue>>,

    /// A page of the most recent timeline events, newest last. Only events that
    /// were world-readable at the point they were sent are included.
    #[salvo(schema(value_type = Vec<Object>))]
    pub messages: Vec<Box<RawJsonValue>>,
}

// ===========================================================================
// MSC2444 ongoing peek subscription.
//
// Unlike the one-shot GET snapshot above, this establishes a *subscription*: the
// resident server records the peeking server and forwards new room events to it
// (via the normal /send transaction path) until the peek expires or is
// cancelled. The peeking server must renew before `renewal_interval` elapses.
//
//   PUT    /_matrix/federation/v1/peek/{room_id}/{peek_id}   start / renew
//   DELETE /_matrix/federation/v1/peek/{room_id}/{peek_id}   cancel
// ===========================================================================

/// Build the request to start or renew an ongoing peek subscription.
pub fn peek_start_request(
    origin: &str,
    room_id: &RoomId,
    peek_id: &str,
    ver: &[RoomVersionId],
) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/peek/{room_id}/{peek_id}"
    ))?;
    for v in ver {
        url.query_pairs_mut().append_pair("ver", v.as_str());
    }
    let mut request = crate::sending::put(url);
    // Empty JSON body, like other federation PUTs.
    *request.body_mut() = Some(b"{}".to_vec().into());
    Ok(request)
}

/// Build the request to cancel an ongoing peek subscription.
pub fn peek_cancel_request(
    origin: &str,
    room_id: &RoomId,
    peek_id: &str,
) -> SendResult<SendRequest> {
    let url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/peek/{room_id}/{peek_id}"
    ))?;
    Ok(crate::sending::delete(url))
}

/// Path/query args for the peek subscription endpoints.
#[derive(ToParameters, Deserialize, Debug)]
pub struct PeekSubReqArgs {
    /// The room to peek.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// A peer-chosen opaque id identifying this peek subscription.
    #[salvo(parameter(parameter_in = Path))]
    pub peek_id: String,

    /// Room versions the peeking server supports (advisory).
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default)]
    pub ver: Vec<RoomVersionId>,
}

/// Response to starting/renewing an ongoing peek. Mirrors a send_join response
/// (full current state + backing auth chain) so the peeking server can build a
/// complete local copy of the room, plus a renewal deadline.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct PeekStartResBody {
    /// The version of the room being peeked.
    pub room_version: RoomVersionId,

    /// The full resolved current state of the room.
    #[salvo(schema(value_type = Vec<Object>))]
    pub state: Vec<Box<RawJsonValue>>,

    /// The auth chain backing `state`, recursively.
    #[salvo(schema(value_type = Vec<Object>))]
    pub auth_chain: Vec<Box<RawJsonValue>>,

    /// A page of recent timeline events, newest last, to seed the timeline.
    #[salvo(schema(value_type = Vec<Object>))]
    pub messages: Vec<Box<RawJsonValue>>,

    /// Milliseconds within which the peeking server must renew, else the
    /// resident server will drop the subscription.
    pub renewal_interval: u64,
}
