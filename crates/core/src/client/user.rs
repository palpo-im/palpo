//! `POST /_matrix/client/*/user/{user_id}/openid/request_token`
//!
//! Request an OpenID 1.0 token to verify identity with a third party.
//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3useruser_idopenidrequest_token

use std::time::Duration;

use salvo::prelude::*;
use serde::Serialize;

use crate::OwnedServerName;
use crate::authentication::TokenType;

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/user/:user_id/openid/request_token",
//         1.1 => "/_matrix/client/v3/user/:user_id/openid/request_token",
//     }
// };

// /// Request type for the `request_openid_token` endpoint.

// pub struct Requesxt {
//     /// User ID of authenticated user.
//     #[salvo(parameter(parameter_in = Path))]
//     pub user_id: OwnedUserId,
// }

/// Response type for the `request_openid_token` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct RequstOpenidTokenResBody {
    /// Access token for verifying user's identity.
    pub access_token: String,

    /// Access token type.
    pub token_type: TokenType,

    /// HomeServer domain for verification of user's identity.
    pub matrix_server_name: OwnedServerName,

    /// Seconds until token expiration.
    #[serde(with = "crate::serde::duration::secs")]
    pub expires_in: Duration,
}
impl RequstOpenidTokenResBody {
    /// Creates a new `Response` with the given access token, token type, server
    /// name and expiration duration.
    pub fn new(
        access_token: String,
        token_type: TokenType,
        matrix_server_name: OwnedServerName,
        expires_in: Duration,
    ) -> Self {
        Self {
            access_token,
            token_type,
            matrix_server_name,
            expires_in,
        }
    }
}
