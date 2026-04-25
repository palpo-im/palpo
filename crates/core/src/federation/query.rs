/// Endpoints to retrieve information from a homeserver about a resource.
///
/// `GET /_matrix/federation/*/query/directory`
///
/// Get mapped room ID and resident homeservers for a given room alias.
use std::collections::BTreeMap;

use reqwest::Url;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::sending::{SendRequest, SendResult};
use crate::user::ProfileField;
use crate::{OwnedRoomId, OwnedServerName, OwnedUserId, RoomAliasId};
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1querydirectory
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/query/directory",
//     }
// };

pub fn directory_request(origin: &str, room_alias: &RoomAliasId) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!("{origin}/_matrix/federation/v1/query/directory"))?;
    url.query_pairs_mut()
        .append_pair("room_alias", room_alias.as_str());
    Ok(crate::sending::get(url))
}

// /// Request type for the `get_room_information` endpoint.

// #[derive(ToSchema, Deserialize, Debug)]
// pub struct RoomInfoReqArgs {
//     /// Room alias to query.
//     #[salvo(parameter(parameter_in = Query))]
//     pub room_alias: OwnedRoomAliasId,
// }

/// Response type for the `get_room_information` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]

pub struct RoomInfoResBody {
    /// Room ID mapped to queried alias.
    pub room_id: OwnedRoomId,

    /// An array of server names that are likely to hold the given room.
    pub servers: Vec<OwnedServerName>,
}
impl RoomInfoResBody {
    /// Creates a new `Response` with the given room IDs and servers.
    pub fn new(room_id: OwnedRoomId, servers: Vec<OwnedServerName>) -> Self {
        Self { room_id, servers }
    }
}

// /// `GET /_matrix/federation/*/query/profile`
// ///
// /// Get profile information, such as a display name or avatar, for a given user.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1queryprofile

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/query/profile",
//     }
// };

pub fn profile_request(origin: &str, args: ProfileReqArgs) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!("{origin}/_matrix/federation/v1/query/profile"))?;
    url.query_pairs_mut()
        .append_pair("user_id", args.user_id.as_str());
    if let Some(field) = &args.field {
        url.query_pairs_mut().append_pair("field", field.as_ref());
    }
    Ok(crate::sending::get(url))
}

/// `GET /_matrix/federation/unstable/io.fsky.vel/edutypes`
///
/// Determine what types of EDUs a server wishes to receive.
pub fn edu_types_request(origin: &str) -> SendResult<SendRequest> {
    let url = Url::parse(&format!(
        "{origin}/_matrix/federation/unstable/io.fsky.vel/edutypes"
    ))?;
    Ok(crate::sending::get(url))
}

/// Request type for the `edutypes` endpoint.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct EduTypesReqBody {}

impl EduTypesReqBody {
    /// Creates a new `EduTypesReqBody`.
    pub fn new() -> Self {
        Self {}
    }
}

/// Response type for the `edutypes` endpoint.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct EduTypesResBody {
    /// Whether presence EDUs should be sent.
    #[serde(rename = "m.presence", default = "crate::serde::default_true")]
    pub presence: bool,

    /// Whether read receipt EDUs should be sent.
    #[serde(rename = "m.receipt", default = "crate::serde::default_true")]
    pub receipt: bool,

    /// Whether typing EDUs should be sent.
    #[serde(rename = "m.typing", default = "crate::serde::default_true")]
    pub typing: bool,
}

impl EduTypesResBody {
    /// Creates a new `EduTypesResBody` with all EDU flags set to `true`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for EduTypesResBody {
    fn default() -> Self {
        Self {
            presence: true,
            receipt: true,
            typing: true,
        }
    }
}

/// Request type for the `get_profile_information` endpoint.

#[derive(ToParameters, Deserialize, Debug)]
pub struct ProfileReqArgs {
    /// User ID to query.
    #[salvo(parameter(parameter_in = Query))]
    pub user_id: OwnedUserId,

    /// Profile field to query.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<ProfileField>,
}

// /// `GET /_matrix/federation/*/query/{queryType}`
// ///
// /// Performs a single query request on the receiving homeserver. The query
// /// arguments are dependent on which type of query is being made.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1queryquerytype

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/federation/v1/query/:query_type",
//     }
// };

/// Request type for the `get_custom_information` endpoint.

#[derive(ToSchema, Deserialize, Debug)]
pub struct CustomReqBody {
    /// The type of query to make.
    #[salvo(parameter(parameter_in = Path))]
    pub query_type: String,

    /// The query parameters.
    pub params: BTreeMap<String, String>,
}

/// Response type for the `get_custom_information` endpoint.
#[derive(ToSchema, Serialize, Debug)]
#[salvo(schema(value_type = Object))]
pub struct CustomResBody(
    /// The body of the response.
    pub JsonValue,
);

impl CustomResBody {
    /// Creates a new response with the given body.
    pub fn new(body: JsonValue) -> Self {
        Self(body)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::EduTypesResBody;

    #[test]
    fn edu_types_default_to_true_when_missing() {
        let body: EduTypesResBody = from_json_value(json!({})).unwrap();

        assert!(body.presence);
        assert!(body.receipt);
        assert!(body.typing);
    }

    #[test]
    fn edu_types_use_msc4373_field_names() {
        let body = EduTypesResBody {
            presence: true,
            receipt: false,
            typing: true,
        };

        assert_eq!(
            to_json_value(body).unwrap(),
            json!({
                "m.presence": true,
                "m.receipt": false,
                "m.typing": true,
            })
        );
    }
}
