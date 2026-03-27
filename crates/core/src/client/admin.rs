//! `GET /_matrix/client/*/admin/whois/{user_id}`
//!
//! Get information about a particular user.
//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3adminwhoisuser_id

use std::collections::BTreeMap;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{OwnedUserId, UnixMillis};

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/admin/whois/:user_id",
//         1.1 => "/_matrix/client/v3/admin/whois/:user_id",
//     }
// };

// /// Request type for the `get_user_info` endpoint.
// #[derive(ToParameters, Deserialize, Debug)]
// pub struct UserInfoReqArgs {
//     /// The user to look up.
//     #[salvo(parameter(parameter_in = Path))]
//     pub user_id: OwnedUserId,
// }

/// Response type for the `get_user_info` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct UserInfoResBody {
    /// The Matrix user ID of the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<OwnedUserId>,

    /// A map of the user's device identifiers to information about that device.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub devices: BTreeMap<String, DeviceInfo>,
}

/// Information about a user's device.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct DeviceInfo {
    /// A list of user sessions on this device.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionInfo>,
}

impl DeviceInfo {
    /// Create a new `DeviceInfo` with no sessions.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Information about a user session.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct SessionInfo {
    /// A list of connections in this session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionInfo>,
}

impl SessionInfo {
    /// Create a new `SessionInfo` with no connections.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Information about a connection in a user session.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct ConnectionInfo {
    /// Most recently seen IP address of the session.
    pub ip: Option<String>,

    /// Time when that the session was last active.
    pub last_seen: Option<UnixMillis>,

    /// User agent string last seen in the session.
    pub user_agent: Option<String>,
}

impl ConnectionInfo {
    /// Create a new `ConnectionInfo` with all fields set to `None`.
    pub fn new() -> Self {
        Self::default()
    }
}
