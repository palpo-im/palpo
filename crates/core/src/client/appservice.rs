/// `POST /_matrix/client/*/appservice/{appserviceId}/ping}`
///
/// Ask the homeserver to ping the application service to ensure the connection
/// works. `/v1/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/application-service-api/#post_matrixclientv1appserviceappserviceidping
use std::time::Duration;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::OwnedTransactionId;
use crate::room::Visibility;

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         unstable =>
// "/_matrix/client/unstable/fi.mau.msc2659/appservice/:appservice_id/ping",
//         1.7 => "/_matrix/client/v1/appservice/:appservice_id/ping",
//     }
// };

/// Request type for the `request_ping` endpoint.

#[derive(ToSchema, Deserialize, Debug)]
pub struct PingReqBody {
    /// The appservice ID of the appservice to ping.
    ///
    /// This must be the same as the appservice whose `as_token` is being used
    /// to authenticate the request.
    #[salvo(parameter(parameter_in = Path))]
    pub appservice_id: String,

    /// Transaction ID that is passed through to the `POST /_matrix/app/v1/ping`
    /// call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<OwnedTransactionId>,
}

/// Response type for the `request_ping` endpoint.

#[derive(ToSchema, Serialize, Debug)]
pub struct PingResBody {
    /// The duration in milliseconds that the `POST /_matrix/app/v1/ping`
    /// request took from the homeserver's point of view.
    #[serde(with = "crate::serde::duration::ms", rename = "duration_ms")]
    pub duration: Duration,
}
impl PingResBody {
    /// Creates an `Response` with the given duration.
    pub fn new(duration: Duration) -> Self {
        Self { duration }
    }
}

// /// `PUT /_matrix/client/*/directory/list/appservice/{networkId}/{room_id}`
// ///
// /// Updates the visibility of a given room on the application service's room
// /// directory.
//
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/application-service-api/#put_matrixclientv3directorylistappservicenetworkidroomid

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/directory/list/appservice/:network_id/:room_id",
//         1.1 => "/_matrix/client/v3/directory/list/appservice/:network_id/:room_id",
//     }
// };

/// Request type for the `set_room_visibility` endpoint.

#[derive(ToSchema, Deserialize, Debug)]
pub struct UpdateRoomReqBody {
    /// Whether the room should be visible (public) in the directory or not
    /// (private).
    pub visibility: Visibility,
}
