//! `PUT /_matrix/client/*/rooms/{room_id}/typing/{user_id}`
//!
//! Send a typing event to a room.
//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3roomsroomidtypinguser_id

use std::time::Duration;

use salvo::oapi::ToSchema;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize};

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     authentication: AccessToken,
//     rate_limited: true,
//     history: {
//         1.0 => "/_matrix/client/r0/rooms/:room_id/typing/:user_id",
//         1.1 => "/_matrix/client/v3/rooms/:room_id/typing/:user_id",
//     }
// };

// /// Request type for the `create_typing_event` endpoint.
// #[derive(ToParameters, Deserialize, Debug)]
// pub struct CreateTypingEventReqArgs {
//     /// The room in which the user is typing.
//     #[salvo(parameter(parameter_in = Path))]
//     pub room_id: OwnedRoomId,

//     /// The user who has started to type.
//     #[salvo(parameter(parameter_in = Path))]
//     pub user_id: OwnedUserId,
// }

/// Request type for the `create_typing_event` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct CreateTypingEventReqBody {
    /// Whether the user is typing within a length of time or not.
    #[serde(flatten)]
    pub state: Typing,
}

/// A mark for whether the user is typing within a length of time or not.
#[derive(ToSchema, Clone, Copy, Debug, Serialize)]
#[serde(into = "TypingInner")]
#[allow(clippy::exhaustive_enums)]
pub enum Typing {
    /// Not typing.
    No,

    /// Typing during the specified length of time.
    Yes(Duration),
}

#[derive(Deserialize, Serialize)]
struct TypingInner {
    typing: bool,

    #[serde(
        with = "crate::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    timeout: Option<Duration>,
}

impl From<Typing> for TypingInner {
    fn from(typing: Typing) -> Self {
        match typing {
            Typing::No => Self {
                typing: false,
                timeout: None,
            },
            Typing::Yes(time) => Self {
                typing: true,
                timeout: Some(time),
            },
        }
    }
}

impl<'de> Deserialize<'de> for Typing {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let inner = TypingInner::deserialize(deserializer)?;

        match (inner.typing, inner.timeout) {
            (false, _) => Ok(Self::No),
            (true, Some(time)) => Ok(Self::Yes(time)),
            _ => Err(D::Error::missing_field("timeout")),
        }
    }
}
