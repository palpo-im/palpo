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

/// A mark for whether the user is typing or not.
#[derive(ToSchema, Clone, Copy, Debug)]
#[allow(clippy::exhaustive_enums)]
pub enum Typing {
    /// The user is currently not typing.
    No,

    /// The user is currently typing.
    Yes(TypingInfo),
}

impl From<TypingInfo> for Typing {
    fn from(value: TypingInfo) -> Self {
        Self::Yes(value)
    }
}

/// Details about the user currently typing.
#[derive(ToSchema, Clone, Copy, Debug)]
#[non_exhaustive]
pub struct TypingInfo {
    /// The length of time to mark this user as typing.
    pub timeout: Duration,
}

impl TypingInfo {
    /// Create a new `TypingInfo` with the given timeout.
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

#[derive(Deserialize, Serialize)]
struct TypingSerdeRepr {
    typing: bool,

    #[serde(
        with = "crate::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    timeout: Option<Duration>,
}

impl Serialize for Typing {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let repr = match self {
            Self::No => TypingSerdeRepr {
                typing: false,
                timeout: None,
            },
            Self::Yes(TypingInfo { timeout }) => TypingSerdeRepr {
                typing: true,
                timeout: Some(*timeout),
            },
        };

        repr.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Typing {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let repr = TypingSerdeRepr::deserialize(deserializer)?;

        Ok(if repr.typing {
            Typing::Yes(TypingInfo {
                timeout: repr.timeout.ok_or_else(|| D::Error::missing_field("timeout"))?,
            })
        } else {
            Typing::No
        })
    }
}
