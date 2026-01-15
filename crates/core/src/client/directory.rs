use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::OwnedServerName;
/// `POST /_matrix/client/*/publicRooms`
///
/// Get the list of rooms in this homeserver's public directory.
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3publicrooms
use crate::directory::{PublicRoomFilter, RoomNetwork};
use crate::room::Visibility;

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/publicRooms",
//         1.1 => "/_matrix/client/v3/publicRooms",
//     }
// };

/// Request type for the `get_filtered_public_rooms` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct PublicRoomsFilteredReqBody {
    /// The server to fetch the public room lists from.
    ///
    /// `None` means the server this request is sent to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub server: Option<OwnedServerName>,

    /// Limit for the number of results to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,

    /// Pagination token from a previous request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,

    /// Filter to apply to the results.
    #[serde(default, skip_serializing_if = "PublicRoomFilter::is_empty")]
    pub filter: PublicRoomFilter,

    /// Network to fetch the public room lists from.
    #[serde(flatten, skip_serializing_if = "crate::serde::is_default")]
    pub room_network: RoomNetwork,
}

// /// `GET /_matrix/client/*/publicRooms`
// ///
// /// Get the list of rooms in this homeserver's public directory.
//
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3publicrooms

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/publicRooms",
//         1.1 => "/_matrix/client/v3/publicRooms",
//     }
// };

/// Request type for the `get_public_rooms` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct PublicRoomsReqArgs {
    /// Limit for the number of results to return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub limit: Option<usize>,

    /// Pagination token from a previous request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub since: Option<String>,

    /// The server to fetch the public room lists from.
    ///
    /// `None` means the server this request is sent to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub server: Option<OwnedServerName>,
}

// /// `GET /_matrix/client/*/directory/list/room/{room_id}`
// ///
// /// Get the visibility of a public room on a directory.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3directorylistroomroomid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/directory/list/room/:room_id",
//         1.1 => "/_matrix/client/v3/directory/list/room/:room_id",
//     }
// };

// /// Request type for the `get_room_visibility` endpoint.

// pub struct Requestx {
//     /// The ID of the room of which to request the visibility.
//     #[salvo(parameter(parameter_in = Path))]
//     pub room_id: OwnedRoomId,
// }

/// Response type for the `get_room_visibility` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct VisibilityResBody {
    /// Visibility of the room.
    pub visibility: Visibility,
}

impl VisibilityResBody {
    /// Creates a new `Response` with the given visibility.
    pub fn new(visibility: Visibility) -> Self {
        Self { visibility }
    }
}

// /// `PUT /_matrix/client/*/directory/list/room/{room_id}`
// ///
// /// Set the visibility of a public room on a directory.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3directorylistroomroomid
// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/directory/list/room/:room_id",
//         1.1 => "/_matrix/client/v3/directory/list/room/:room_id",
//     }
// };

/// Request type for the `set_room_visibility` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct SetRoomVisibilityReqBody {
    // /// The ID of the room of which to set the visibility.
    // #[salvo(parameter(parameter_in = Path))]
    // pub room_id: OwnedRoomId,
    /// New visibility setting for the room.
    pub visibility: Visibility,
}
