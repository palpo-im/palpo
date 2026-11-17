/// `PUT /_matrix/client/*/presence/{user_id}/status`
///
/// Set presence status for this user.
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3presenceuser_idstatus
use std::time::Duration;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::presence::PresenceState;

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/presence/:user_id/status",
//         1.1 => "/_matrix/client/v3/presence/:user_id/status",
//     }
// };

/// Request type for the `set_presence` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct SetPresenceReqBody {
    // /// The user whose presence state will be updated.
    // #[salvo(parameter(parameter_in = Path))]
    // pub user_id: OwnedUserId,
    /// The new presence state.
    pub presence: PresenceState,

    /// The status message to attach to this state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_msg: Option<String>,
}

// /// `GET /_matrix/client/*/presence/{user_id}/status`
// ///
// /// Get presence status for this user.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3presenceuser_idstatus

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/presence/:user_id/status",
//         1.1 => "/_matrix/client/v3/presence/:user_id/status",
//     }
// };

// /// Request type for the `get_presence` endpoint.

// pub struct Reqxuest {
//     /// The user whose presence state will be retrieved.
//     #[salvo(parameter(parameter_in = Path))]
//     pub user_id: OwnedUserId,
// }

/// Response type for the `get_presence` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct PresenceResBody {
    /// The state message for this user if one was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_msg: Option<String>,

    /// Whether or not the user is currently active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currently_active: Option<bool>,

    /// The length of time in milliseconds since an action was performed by the
    /// user.
    #[serde(
        with = "crate::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[salvo(schema(value_type = Option<u64>))]
    pub last_active_ago: Option<Duration>,

    /// The user's presence state.
    pub presence: PresenceState,
}
impl PresenceResBody {
    /// Creates a new `Response` with the given presence state.
    pub fn new(presence: PresenceState) -> Self {
        Self {
            presence,
            status_msg: None,
            currently_active: None,
            last_active_ago: None,
        }
    }
}
