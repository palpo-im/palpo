use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::tag::{TagInfo, Tags};
use crate::{OwnedRoomId, OwnedUserId};

// /// `GET /_matrix/client/*/user/{user_id}/rooms/{room_id}/tags`
// ///
// /// Get the tags associated with a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3useruser_idroomsroomidtags
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/user/:user_id/rooms/:room_id/tags",
//         1.1 => "/_matrix/client/v3/user/:user_id/rooms/:room_id/tags",
//     }
// };

// /// Request type for the `get_tags` endpoint.
// #[derive(ToParameters, Deserialize, Debug)]
// pub struct TagsReqArgs {
//     /// The user whose tags will be retrieved.
//     #[salvo(parameter(parameter_in = Path))]
//     pub user_id: OwnedUserId,

//     /// The room from which tags will be retrieved.
//     #[salvo(parameter(parameter_in = Path))]
//     pub room_id: OwnedRoomId,
// }

/// Response type for the `get_tags` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct TagsResBody {
    /// The user's tags for the room.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub tags: Tags,
}
impl TagsResBody {
    /// Creates a new `Response` with the given tags.
    pub fn new(tags: Tags) -> Self {
        Self { tags }
    }
}

// /// `DELETE /_matrix/client/*/user/{user_id}/rooms/{room_id}/tags/{tag}`
// ///
// /// Remove a tag from a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3useruser_idroomsroomidtagstag

// const METADATA: Metadata = metadata! {
//     method: DELETE,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/user/:user_id/rooms/:room_id/tags/:tag",
//         1.1 => "/_matrix/client/v3/user/:user_id/rooms/:room_id/tags/:tag",
//     }
// };

// /// Request type for the `delete_tag` endpoint.

// pub struct DeleteTagReqBody {
//     /// The user whose tag will be deleted.
//     #[salvo(parameter(parameter_in = Path))]
//     pub user_id: OwnedUserId,

//     /// The tagged room.
//     #[salvo(parameter(parameter_in = Path))]
//     pub room_id: OwnedRoomId,

//     /// The name of the tag to delete.
//     #[salvo(parameter(parameter_in = Path))]
//     pub tag: String,
// }

// /// `PUT /_matrix/client/*/user/{user_id}/rooms/{room_id}/tags/{tag}`
// ///
// /// Add a new tag to a room.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3useruser_idroomsroomidtagstag

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/user/:user_id/rooms/:room_id/tags/:tag",
//         1.1 => "/_matrix/client/v3/user/:user_id/rooms/:room_id/tags/:tag",
//     }
// };

/// Request args for the `create_tag` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct OperateTagReqArgs {
    /// The ID of the user creating the tag.
    #[salvo(parameter(parameter_in = Path))]
    pub user_id: OwnedUserId,

    /// The room to tag.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The name of the tag to create.
    #[salvo(parameter(parameter_in = Path))]
    pub tag: String,
}
/// Request type for the `create_tag` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct UpsertTagReqBody {
    /// Info about the tag.
    pub tag_info: TagInfo,
}
