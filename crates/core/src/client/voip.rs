//! `GET /_matrix/client/*/voip/turnServer`
//!
//! Get credentials for the client to use when initiating VoIP calls.
//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3voipturnserver

use std::time::Duration;

use salvo::prelude::*;
use serde::Serialize;

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/voip/turnServer",
//         1.1 => "/_matrix/client/v3/voip/turnServer",
//     }
// };

/// Response type for the `turn_server_info` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct TurnServerResBody {
    /// The username to use.
    pub username: String,

    /// The password to use.
    pub password: String,

    /// A list of TURN URIs.
    pub uris: Vec<String>,

    /// The time-to-live in seconds.
    #[serde(with = "palpo_core::serde::duration::secs")]
    #[salvo(schema(value_type = u64))]
    pub ttl: Duration,
}

impl TurnServerResBody {
    /// Creates a new `Response` with the given username, password, TURN URIs
    /// and time-to-live.
    pub fn new(username: String, password: String, uris: Vec<String>, ttl: Duration) -> Self {
        Self {
            username,
            password,
            uris,
            ttl,
        }
    }
}
