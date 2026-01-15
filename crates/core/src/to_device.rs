use std::collections::BTreeMap;
/// Common types for the Send-To-Device Messaging
///
/// [send-to-device]: https://spec.matrix.org/latest/client-server-api/#send-to-device-messaging
use std::fmt::{Display, Formatter, Result as FmtResult};

use salvo::prelude::*;
use serde::de::{self, Unexpected};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::events::{AnyToDeviceEventContent, ToDeviceEventType};
use crate::serde::RawJson;
use crate::{OwnedDeviceId, OwnedTransactionId, OwnedUserId};

/// Represents one or all of a user's devices.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[allow(clippy::exhaustive_enums)]
pub enum DeviceIdOrAllDevices {
    /// Represents a device Id for one of a user's devices.
    DeviceId(OwnedDeviceId),

    /// Represents all devices for a user.
    AllDevices,
}

impl Display for DeviceIdOrAllDevices {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            DeviceIdOrAllDevices::DeviceId(device_id) => write!(f, "{device_id}"),
            DeviceIdOrAllDevices::AllDevices => write!(f, "*"),
        }
    }
}

impl From<OwnedDeviceId> for DeviceIdOrAllDevices {
    fn from(d: OwnedDeviceId) -> Self {
        DeviceIdOrAllDevices::DeviceId(d)
    }
}

impl TryFrom<&str> for DeviceIdOrAllDevices {
    type Error = &'static str;

    fn try_from(device_id_or_all_devices: &str) -> Result<Self, Self::Error> {
        if device_id_or_all_devices.is_empty() {
            Err("Device identifier cannot be empty")
        } else if "*" == device_id_or_all_devices {
            Ok(DeviceIdOrAllDevices::AllDevices)
        } else {
            Ok(DeviceIdOrAllDevices::DeviceId(
                device_id_or_all_devices.into(),
            ))
        }
    }
}

impl Serialize for DeviceIdOrAllDevices {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::DeviceId(device_id) => device_id.serialize(serializer),
            Self::AllDevices => serializer.serialize_str("*"),
        }
    }
}

impl<'de> Deserialize<'de> for DeviceIdOrAllDevices {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = crate::serde::deserialize_cow_str(deserializer)?;
        DeviceIdOrAllDevices::try_from(s.as_ref()).map_err(|_| {
            de::Error::invalid_value(Unexpected::Str(&s), &"a valid device identifier or '*'")
        })
    }
}
/// `PUT /_matrix/client/*/sendToDevice/{eventType}/{txn_id}`
///
/// Send an event to a device or devices.
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3sendtodeviceeventtypetxnid

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/sendToDevice/:event_type/:txn_id",
//         1.1 => "/_matrix/client/v3/sendToDevice/:event_type/:txn_id",
//     }
// };

#[derive(ToParameters, Deserialize, Debug)]
pub struct SendEventToDeviceReqArgs {
    /// Type of event being sent to each device.
    #[salvo(parameter(parameter_in = Path))]
    pub event_type: ToDeviceEventType,

    /// The transaction ID for this event.
    ///
    /// Clients should generate a unique ID across requests within the
    /// same session. A session is identified by an access token, and
    /// persists when the [access token is refreshed].
    ///
    /// It will be used by the server to ensure idempotency of requests.
    ///
    /// [access token is refreshed]: https://spec.matrix.org/latest/client-server-api/#refreshing-access-tokens
    #[salvo(parameter(parameter_in = Path))]
    pub txn_id: OwnedTransactionId,
}

/// Request type for the `send_event_to_device` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct SendEventToDeviceReqBody {
    /// Messages to send.
    ///
    /// Different message events can be sent to different devices in the same
    /// request, but all events within one request must be of the same type.
    #[salvo(schema(value_type = Object))]
    pub messages: Messages,
}

/// Messages to send in a send-to-device request.
///
/// Represented as a map of `{ user-ids => { device-ids => message-content } }`.
pub type Messages =
    BTreeMap<OwnedUserId, BTreeMap<DeviceIdOrAllDevices, RawJson<AnyToDeviceEventContent>>>;
