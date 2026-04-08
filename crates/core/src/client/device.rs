use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::client::uiaa::AuthData;
/// Endpoints for managing devices.
use crate::{OwnedDeviceId, UnixMillis};

/// Information about a registered device.
#[derive(ToSchema, Clone, Debug, Deserialize, Hash, Serialize)]
pub struct Device {
    /// Device ID
    pub device_id: OwnedDeviceId,

    /// Public display name of the device.
    pub display_name: Option<String>,

    /// Most recently seen IP address of the session.
    pub last_seen_ip: Option<String>,

    /// Unix timestamp that the session was last active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_ts: Option<UnixMillis>,
}

impl Device {
    /// Creates a new `Device` with the given device ID.
    pub fn new(device_id: OwnedDeviceId) -> Self {
        Self {
            device_id,
            display_name: None,
            last_seen_ip: None,
            last_seen_ts: None,
        }
    }
}

// /// `DELETE /_matrix/client/*/devices/{deviceId}`
// ///
// /// Delete a device for authenticated user.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#delete_matrixclientv3devicesdeviceid

// const METADATA: Metadata = metadata! {
//     method: DELETE,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/devices/:device_id",
//         1.1 => "/_matrix/client/v3/devices/:device_id",
//     }
// };

/// Request type for the `delete_device` endpoint.
#[derive(ToSchema, Deserialize, Default, Debug)]
pub struct DeleteDeviceReqBody {
    /// Additional authentication information for the user-interactive
    /// authentication API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthData>,
}

// /// `POST /_matrix/client/*/delete_devices`
// ///
// /// Delete specified devices.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3delete_devices

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/delete_devices",
//         1.1 => "/_matrix/client/v3/delete_devices",
//     }
// };

/// Request type for the `delete_devices` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct DeleteDevicesReqBody {
    /// List of devices to delete.
    pub devices: Vec<OwnedDeviceId>,

    /// Additional authentication information for the user-interactive
    /// authentication API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthData>,
}

// /// `GET /_matrix/client/*/devices/{deviceId}`
// ///
// /// Get a device for authenticated user.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3devicesdeviceid

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/devices/:device_id",
//         1.1 => "/_matrix/client/v3/devices/:device_id",
//     }
// };

// /// Request type for the `get_device` endpoint.

// pub struct Rexquest {
//     /// The device to retrieve.
//     #[salvo(parameter(parameter_in = Path))]
//     pub device_id: OwnedDeviceId,
// }

/// Response type for the `get_device` endpoint.

#[derive(ToSchema, Serialize, Debug)]

pub struct DeviceResBody(pub Device);
impl DeviceResBody {
    /// Creates a new `Response` with the given device.
    pub fn new(device: Device) -> Self {
        Self(device)
    }
}

// /// `GET /_matrix/client/*/devices`
// ///
// /// Get registered devices for authenticated user.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3devices
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/devices",
//         1.1 => "/_matrix/client/v3/devices",
//     }
// };

/// Response type for the `get_devices` endpoint.

#[derive(ToSchema, Serialize, Debug)]
pub struct DevicesResBody {
    /// A list of all registered devices for this user
    pub devices: Vec<Device>,
}

// impl Response {
//     /// Creates a new `Response` with the given devices.
//     pub fn new(devices: Vec<Device>) -> Self {
//         Self { devices }
//     }
// }

// /// `PUT /_matrix/client/*/devices/{deviceId}`
// ///
// /// Update metadata for a device, or create a new device.
// ///
// /// Only application services can use this endpoint to create new devices.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3devicesdeviceid

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/devices/:device_id",
//         1.1 => "/_matrix/client/v3/devices/:device_id",
//     }
// };

/// Request type for the `update_device` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct UpdatedDeviceReqBody {
    /// The new display name for this device.
    ///
    /// If this is `None`, the display name won't be changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}
