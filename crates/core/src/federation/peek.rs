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
    let mut url = Url::parse(&format!("{origin}/_matrix/federation/v1/peek/{}", args.room_id))?;
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

    /// The auth chain backing the returned room state, recursively.
    #[salvo(schema(value_type = Vec<Object>))]
    pub auth_chain: Vec<Box<RawJsonValue>>,

    /// The current resolved state of the room.
    #[salvo(schema(value_type = Vec<Object>))]
    pub pdus: Vec<Box<RawJsonValue>>,

    /// A page of the most recent timeline events, newest last.
    #[salvo(schema(value_type = Vec<Object>))]
    pub messages: Vec<Box<RawJsonValue>>,
}
