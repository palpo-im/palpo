//! Endpoints to retrieve information about user devices

//! `GET /_matrix/federation/*/user/devices/{user_id}`
//!
//! Get information about a user's devices.
//! `/v1/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1userdevicesuser_id

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::encryption::{CrossSigningKey, DeviceKeys};
use crate::{OwnedDeviceId, OwnedUserId};
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: ServerSignatures,
//     history: {
//         1.0 => "/_matrix/federation/v1/user/devices/:user_id",
//     }
// };

// /// Request type for the `get_devices` endpoint.

// pub struct DeviceReqBody {
//     /// The user ID to retrieve devices for.
//     ///
//     /// Must be a user local to the receiving homeserver.
//     #[salvo(parameter(parameter_in = Path))]
//     pub user_id: OwnedUserId,
// }

/// Response type for the `get_devices` endpoint.
#[derive(ToSchema, Serialize, Debug)]

pub struct DevicesResBody {
    /// The user ID devices were requested for.
    pub user_id: OwnedUserId,

    /// A unique ID for a given user_id which describes the version of the
    /// returned device list.
    ///
    /// This is matched with the `stream_id` field in `m.device_list_update`
    /// EDUs in order to incrementally update the returned device_list.
    pub stream_id: u64,

    /// The user's devices.
    pub devices: Vec<Device>,

    /// The user's master key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_key: Option<CrossSigningKey>,

    /// The users's self-signing key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_signing_key: Option<CrossSigningKey>,
}
impl DevicesResBody {
    /// Creates a new `Response` with the given user id and stream id.
    ///
    /// The device list will be empty.
    pub fn new(user_id: OwnedUserId, stream_id: u64) -> Self {
        Self {
            user_id,
            stream_id,
            devices: Vec::new(),
            master_key: None,
            self_signing_key: None,
        }
    }
}

/// Information about a user's device.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    /// The device ID.
    pub device_id: OwnedDeviceId,

    /// Identity keys for the device.
    pub keys: DeviceKeys,

    /// Optional display name for the device
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_display_name: Option<String>,
}

impl Device {
    /// Creates a new `Device` with the given device id and keys.
    pub fn new(device_id: OwnedDeviceId, keys: DeviceKeys) -> Self {
        Self {
            device_id,
            keys,
            device_display_name: None,
        }
    }
}
