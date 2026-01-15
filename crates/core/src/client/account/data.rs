use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::events::{AnyGlobalAccountDataEventContent, AnyRoomAccountDataEventContent};
use crate::serde::RawJson;

/// Response type for the `get_global_account_data` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct GlobalAccountDataResBody(
    /// Account data content for the given type.
    ///
    /// Since the inner type of the `RawJson` does not implement `Deserialize`,
    /// you need to use `.deserialize_as::<T>()` or
    /// `.cast_ref::<T>().deserialize_with_type()` for event types with a
    /// variable suffix (like [`SecretStorageKeyEventContent`]) to
    /// deserialize it.
    ///
    /// [`SecretStorageKeyEventContent`]: palpo_core::events::secret_storage::key::SecretStorageKeyEventContent
    // #[salvo(schema(value_type = Object, additional_properties = true))]
    // #[serde(flatten)]
    pub  RawJson<AnyGlobalAccountDataEventContent>,
);

/// Response type for the `get_room_account_data` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct RoomAccountDataResBody(
    /// Account data content for the given type.
    ///
    /// Since the inner type of the `RawJson` does not implement `Deserialize`,
    /// you need to use `.deserialize_as::<T>()` or
    /// `.cast_ref::<T>().deserialize_with_type()` for event types with a
    /// variable suffix (like [`SecretStorageKeyEventContent`]) to
    /// deserialize it.
    ///
    /// [`SecretStorageKeyEventContent`]: palpo_core::events::secret_storage::key::SecretStorageKeyEventContent
    // #[salvo(schema(value_type = Object, additional_properties = true))]
    // #[serde(flatten)]
    pub  RawJson<AnyRoomAccountDataEventContent>,
);

/// `PUT /_matrix/client/*/user/{user_id}/account_data/{type}`
///
/// Sets global account data.
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3useruser_idaccount_datatype

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/user/:user_id/account_data/:event_type",
//         1.1 => "/_matrix/client/v3/user/:user_id/account_data/:event_type",
//     }
// };

#[derive(ToSchema, Deserialize, Debug)]
#[salvo(schema(value_type = Object))]
pub struct SetGlobalAccountDataReqBody(
    /// Arbitrary JSON to store as config data.
    ///
    /// To create a `RawJsonValue`, use `serde_json::value::to_raw_value`.
    pub RawJson<AnyGlobalAccountDataEventContent>,
);

/// Request type for the `set_room_account_data` endpoint.

#[derive(ToSchema, Deserialize, Debug)]
pub struct SetRoomAccountDataReqBody {
    /// Arbitrary JSON to store as config data.
    ///
    /// To create a `RawJsonValue`, use `serde_json::value::to_raw_value`.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub data: RawJson<AnyRoomAccountDataEventContent>,
}
