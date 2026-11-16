//! Wire types for `POST
//! /_matrix/federation/unstable/org.continuwuity.msc4560.stripped_state/{room_id}`.
//!
//! The current proposal permits unsigned anonymous requests as well as signed
//! requests. Authentication and visibility must be enforced by the future
//! server route; parsing this body does not authorize access.

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::OwnedRoomId;
use crate::events::AnyStrippedStateEvent;
use crate::serde::RawJson;
pub use crate::stripped_state::StrippedStateMsc4560ReqBody;

#[derive(ToParameters, Debug, Deserialize)]
pub struct StrippedStateMsc4560ReqArgs {
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,
}

/// Federation responses carry no supplying-server field.
#[derive(ToSchema, Debug, Default, Deserialize, Serialize)]
pub struct StrippedStateMsc4560ResBody {
    pub state: Vec<RawJson<AnyStrippedStateEvent>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_joined_members: Option<u64>,
}
