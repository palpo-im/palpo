/// Endpoints for querying user IDs and room aliases
///
/// `GET /_matrix/app/*/rooms/{roomAlias}`
///
/// Endpoint to query the existence of a given room alias.
/// `/v1/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/application-service-api/#get_matrixappv1roomsroomalias
use salvo::oapi::ToParameters;
use serde::Deserialize;
use url::Url;

use crate::OwnedRoomAliasId;
use crate::sending::{SendRequest, SendResult};

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/app/v1/rooms/:room_alias",
//     }
// };

pub fn query_room_alias_request(
    origin: &str,
    args: QueryRoomAliasReqArgs,
) -> SendResult<SendRequest> {
    let url = Url::parse(&format!(
        "{origin}/_matrix/app/v1/rooms/{}",
        args.room_alias
    ))?;
    Ok(crate::sending::post(url))
}

/// Request type for the `query_room_alias` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct QueryRoomAliasReqArgs {
    /// The room alias being queried.
    #[salvo(parameter(parameter_in = Path))]
    pub room_alias: OwnedRoomAliasId,
}

// /// `GET /_matrix/app/*/users/{user_id}`
// ///  "/_matrix/app/v1/users/:user_id
// ///
// /// Endpoint to query the existence of a given user ID.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/application-service-api/#get_matrixappv1usersuser_id

// /// Request type for the `query_user_id` endpoint.

// #[derive(ToParameters, Deserialize, Debug)]
// pub struct QueryUseridReqArgs {
//     /// The user ID being queried.
//     #[salvo(parameter(parameter_in = Path))]
//     pub user_id: OwnedUserId,
// }
