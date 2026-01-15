use salvo::oapi::{ToParameters, ToSchema};
use serde::{Deserialize, Serialize};

use crate::events::GlobalAccountDataEventType;
use crate::serde::StringEnum;
use crate::{OwnedMxcUri, OwnedRoomId, OwnedUserId, PrivOwnedStr};

/// Profile fields to specify in query.
///
/// This type can hold an arbitrary string. To build this with a custom value,
/// convert it from a string with `::from()` / `.into()`. To check for values
/// that are not available as a documented variant here, use its string
/// representation, obtained through [`.as_str()`](Self::as_str()).
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
pub enum ProfileField {
    /// Display name of the user.
    #[palpo_enum(rename = "displayname")]
    DisplayName,

    /// Avatar URL for the user's avatar.
    #[palpo_enum(rename = "avatar_url")]
    AvatarUrl,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct UserRoomReqArgs {
    /// The user whose tags will be retrieved.
    #[salvo(parameter(parameter_in = Path))]
    pub user_id: OwnedUserId,

    /// The room from which tags will be retrieved.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct UserEventTypeReqArgs {
    /// The ID of the user to set account_data for.
    ///
    /// The access token must be authorized to make requests for this user ID.
    #[salvo(parameter(parameter_in = Path))]
    pub user_id: OwnedUserId,

    /// The event type of the account_data to set.
    ///
    /// Custom types should be namespaced to avoid clashes.
    #[salvo(parameter(parameter_in = Path))]
    pub event_type: GlobalAccountDataEventType,
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct UserRoomEventTypeReqArgs {
    /// The ID of the user to set account_data for.
    ///
    /// The access token must be authorized to make requests for this user ID.
    #[salvo(parameter(parameter_in = Path))]
    pub user_id: OwnedUserId,

    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The event type of the account_data to set.
    ///
    /// Custom types should be namespaced to avoid clashes.
    #[salvo(parameter(parameter_in = Path))]
    pub event_type: GlobalAccountDataEventType,
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct UserFilterReqArgs {
    /// The user ID to download a filter for.
    #[salvo(parameter(parameter_in = Path))]
    pub user_id: OwnedUserId,

    /// The ID of the filter to download.
    #[salvo(parameter(parameter_in = Path))]
    pub filter_id: String,
}

// ///  GET /_matrix/federation/v1/query/profile
// /// `GET /_matrix/client/*/profile/{user_id}`
// ///
// /// Get all profile information of an user.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3profileuser_id
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/profile/:user_id",
//         1.1 => "/_matrix/client/v3/profile/:user_id",
//     }
// };

// /// Request type for the `get_profile` endpoint.

/// Response type for the `get_profile` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Default, Debug)]
pub struct ProfileResBody {
    /// The user's avatar URL, if set.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "crate::serde::empty_string_as_none"
    )]
    pub avatar_url: Option<OwnedMxcUri>,

    /// The user's display name, if set.
    #[serde(skip_serializing_if = "Option::is_none", rename = "displayname")]
    pub display_name: Option<String>,

    /// The [BlurHash](https://blurha.sh) for the avatar pointed to by `avatar_url`.
    ///
    /// This uses the unstable prefix in
    /// [MSC2448](https://github.com/matrix-org/matrix-spec-proposals/pull/2448).
    #[serde(
        rename = "xyz.amorgan.blurhash",
        skip_serializing_if = "Option::is_none"
    )]
    pub blurhash: Option<String>,
}
impl ProfileResBody {
    /// Creates a new `Response` with the given avatar URL and display name.
    pub fn new(avatar_url: Option<OwnedMxcUri>, display_name: Option<String>) -> Self {
        Self {
            avatar_url,
            display_name,
            blurhash: None,
        }
    }
}
