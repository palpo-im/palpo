//! Endpoints for managing dehydrated devices.

use std::collections::BTreeMap;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::encryption::{DeviceKeys, OneTimeKey};
use crate::serde::StringEnum;
use crate::{OwnedDeviceId, OwnedDeviceKeyId, PrivOwnedStr};

/// Data for a dehydrated device.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "Helper", into = "Helper")]
pub enum DehydratedDeviceData {
    /// The `org.matrix.msc3814.v1.olm` variant of a dehydrated device.
    V1(DehydratedDeviceV1),

    /// The `org.matrix.msc3814.v2` variant of a dehydrated device.
    V2(DehydratedDeviceV2),
}

impl DehydratedDeviceData {
    /// Get the algorithm this dehydrated device uses.
    pub fn algorithm(&self) -> DeviceDehydrationAlgorithm {
        match self {
            DehydratedDeviceData::V1(_) => DeviceDehydrationAlgorithm::V1,
            DehydratedDeviceData::V2(_) => DeviceDehydrationAlgorithm::V2,
        }
    }
}

/// The `org.matrix.msc3814.v1.olm` variant of a dehydrated device.
#[derive(Clone, Debug)]
pub struct DehydratedDeviceV1 {
    /// The pickle of the `Olm` account of the device.
    ///
    /// The pickle will contain the private parts of the long-term identity keys
    /// of the device as well as a collection of one-time keys.
    pub device_pickle: String,
}

impl DehydratedDeviceV1 {
    /// Create a [`DehydratedDeviceV1`] struct from a device pickle.
    pub fn new(device_pickle: String) -> Self {
        Self { device_pickle }
    }
}

/// The `org.matrix.msc3814.v2` variant of a dehydrated device.
#[derive(Clone, Debug)]
pub struct DehydratedDeviceV2 {
    /// The pickle of the `Olm` account of the device.
    ///
    /// The pickle will contain the private parts of the long-term identity keys
    /// of the device as well as a collection of one-time keys.
    pub device_pickle: String,

    /// The nonce used to encrypt the pickle.
    pub nonce: String,
}

impl DehydratedDeviceV2 {
    /// Create a [`DehydratedDeviceV2`] struct from a device pickle.
    pub fn new(device_pickle: String, nonce: String) -> Self {
        Self {
            device_pickle,
            nonce,
        }
    }
}

/// The algorithms used for dehydrated devices.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
pub enum DeviceDehydrationAlgorithm {
    /// The `org.matrix.msc3814.v1.olm` device dehydration algorithm.
    #[palpo_enum(rename = "org.matrix.msc3814.v1.olm")]
    V1,
    /// The `org.matrix.msc3814.v2` device dehydration algorithm.
    #[palpo_enum(rename = "org.matrix.msc3814.v2")]
    V2,
    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// Request type for storing a dehydrated device.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct UpsertDehydratedDeviceReqBody {
    /// The ID of the dehydrated device.
    pub device_id: OwnedDeviceId,

    /// The dehydrated device payload.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub device_data: DehydratedDeviceData,

    /// Identity keys for the dehydrated device.
    pub device_keys: DeviceKeys,

    /// The human-readable display name for the dehydrated device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_device_display_name: Option<String>,

    /// One-time public keys for "pre-key" messages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub one_time_keys: BTreeMap<OwnedDeviceKeyId, OneTimeKey>,

    /// Fallback public keys for "pre-key" messages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fallback_keys: BTreeMap<OwnedDeviceKeyId, OneTimeKey>,
}

/// Response type for storing a dehydrated device.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct UpsertDehydratedDeviceResBody {
    /// The ID of the stored dehydrated device.
    pub device_id: OwnedDeviceId,
}

impl UpsertDehydratedDeviceResBody {
    /// Creates a response for the stored dehydrated device.
    pub fn new(device_id: OwnedDeviceId) -> Self {
        Self { device_id }
    }
}

/// Response type for deleting a dehydrated device.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct DeleteDehydratedDeviceResBody {
    /// The ID of the deleted dehydrated device.
    pub device_id: OwnedDeviceId,
}

impl DeleteDehydratedDeviceResBody {
    /// Creates a response for the deleted dehydrated device.
    pub fn new(device_id: OwnedDeviceId) -> Self {
        Self { device_id }
    }
}

/// Response type for retrieving a dehydrated device.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct GetDehydratedDeviceResBody {
    /// The ID of the dehydrated device.
    pub device_id: OwnedDeviceId,

    /// The dehydrated device payload.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub device_data: DehydratedDeviceData,
}

impl GetDehydratedDeviceResBody {
    /// Creates a response with the stored dehydrated device.
    pub fn new(device_id: OwnedDeviceId, device_data: DehydratedDeviceData) -> Self {
        Self {
            device_id,
            device_data,
        }
    }
}

#[derive(Deserialize, Serialize)]
struct Helper {
    algorithm: DeviceDehydrationAlgorithm,
    device_pickle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
}

impl TryFrom<Helper> for DehydratedDeviceData {
    type Error = serde_json::Error;

    fn try_from(value: Helper) -> Result<Self, Self::Error> {
        match value.algorithm {
            DeviceDehydrationAlgorithm::V1 => Ok(DehydratedDeviceData::V1(DehydratedDeviceV1 {
                device_pickle: value.device_pickle,
            })),
            DeviceDehydrationAlgorithm::V2 => Ok(DehydratedDeviceData::V2(DehydratedDeviceV2 {
                device_pickle: value.device_pickle,
                nonce: value.nonce.ok_or(serde::de::Error::custom(
                    "Missing nonce in v2 dehydrated device.",
                ))?,
            })),
            _ => Err(serde::de::Error::custom(
                "Unsupported device dehydration algorithm.",
            )),
        }
    }
}

impl From<DehydratedDeviceData> for Helper {
    fn from(value: DehydratedDeviceData) -> Self {
        let algorithm = value.algorithm();

        match value {
            DehydratedDeviceData::V1(d) => Self {
                algorithm,
                device_pickle: d.device_pickle,
                nonce: None,
            },
            DehydratedDeviceData::V2(d) => Self {
                algorithm,
                device_pickle: d.device_pickle,
                nonce: Some(d.nonce),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::{DehydratedDeviceData, UpsertDehydratedDeviceReqBody};

    #[test]
    fn deserialize_v1_dehydrated_device_data() {
        let data = from_json_value::<DehydratedDeviceData>(json!({
            "algorithm": "org.matrix.msc3814.v1.olm",
            "device_pickle": "pickle",
        }))
        .unwrap();

        assert!(matches!(data, DehydratedDeviceData::V1(_)));
    }

    #[test]
    fn round_trip_v2_dehydrated_device_data() {
        let data = from_json_value::<DehydratedDeviceData>(json!({
            "algorithm": "org.matrix.msc3814.v2",
            "device_pickle": "pickle",
            "nonce": "nonce",
        }))
        .unwrap();

        let DehydratedDeviceData::V2(v2) = data else {
            panic!("expected v2 dehydrated device data");
        };
        assert_eq!(v2.device_pickle, "pickle");
        assert_eq!(v2.nonce, "nonce");

        let serialized = to_json_value(DehydratedDeviceData::V2(v2)).unwrap();
        assert_eq!(serialized["algorithm"], "org.matrix.msc3814.v2");
        assert_eq!(serialized["nonce"], "nonce");
    }

    #[test]
    fn reject_v2_dehydrated_device_without_nonce() {
        let error = from_json_value::<DehydratedDeviceData>(json!({
            "algorithm": "org.matrix.msc3814.v2",
            "device_pickle": "pickle",
        }))
        .unwrap_err();

        assert!(error.to_string().contains("Missing nonce"));
    }

    #[test]
    fn reject_upsert_request_without_device_keys() {
        let error = from_json_value::<UpsertDehydratedDeviceReqBody>(json!({
            "device_id": "DEHYDRATED",
            "device_data": {
                "algorithm": "org.matrix.msc3814.v1.olm",
                "device_pickle": "pickle",
            },
        }))
        .unwrap_err();

        assert!(error.to_string().contains("device_keys"));
    }
}
