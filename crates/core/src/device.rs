use std::collections::BTreeMap;

use salvo::prelude::*;
use serde::{self, Deserialize, Serialize};

use crate::encryption::DeviceKeys;
use crate::events::{AnyToDeviceEventContent, ToDeviceEventType};
use crate::serde::RawJson;
use crate::to_device::DeviceIdOrAllDevices;
use crate::{OwnedDeviceId, OwnedTransactionId, OwnedUserId};

/// Information on E2E device updates.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct DeviceLists {
    /// List of users who have updated their device identity keys or who now
    /// share an encrypted room with the client since the previous sync.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<OwnedUserId>,

    /// List of users who no longer share encrypted rooms since the previous
    /// sync response.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<OwnedUserId>,
}

impl DeviceLists {
    /// Creates an empty `DeviceLists`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns true if there are no device list updates.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.left.is_empty()
    }
}

/// The description of the direct-to- device message.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct DeviceListUpdateContent {
    /// The user ID who owns the device.
    pub user_id: OwnedUserId,

    /// The ID of the device whose details are changing.
    pub device_id: OwnedDeviceId,

    /// The public human-readable name of this device.
    ///
    /// Will be absent if the device has no name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_display_name: Option<String>,

    /// An ID sent by the server for this update, unique for a given user_id.
    pub stream_id: u64,

    /// The stream_ids of any prior m.device_list_update EDUs sent for this user
    /// which have not been referred to already in an EDU's prev_id field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prev_id: Vec<u64>,

    /// True if the server is announcing that this device has been deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,

    /// The updated identity keys (if any) for this device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<RawJson<DeviceKeys>>,
}

impl DeviceListUpdateContent {
    /// Create a new `DeviceListUpdateContent` with the given `user_id`,
    /// `device_id` and `stream_id`.
    pub fn new(user_id: OwnedUserId, device_id: OwnedDeviceId, stream_id: u64) -> Self {
        Self {
            user_id,
            device_id,
            device_display_name: None,
            stream_id,
            prev_id: vec![],
            deleted: None,
            keys: None,
        }
    }
}
/// The description of the direct-to- device message.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct DirectDeviceContent {
    /// The user ID of the sender.
    pub sender: OwnedUserId,

    /// Event type for the message.
    #[serde(rename = "type")]
    pub ev_type: ToDeviceEventType,

    /// Unique utf8 string ID for the message, used for idempotency.
    pub message_id: OwnedTransactionId,

    /// The contents of the messages to be sent.
    ///
    /// These are arranged in a map of user IDs to a map of device IDs to
    /// message bodies. The device ID may also be *, meaning all known
    /// devices for the user.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub messages: DirectDeviceMessages,
}

impl DirectDeviceContent {
    /// Creates a new `DirectDeviceContent` with the given `sender, `ev_type`
    /// and `message_id`.
    pub fn new(
        sender: OwnedUserId,
        ev_type: ToDeviceEventType,
        message_id: OwnedTransactionId,
    ) -> Self {
        Self {
            sender,
            ev_type,
            message_id,
            messages: DirectDeviceMessages::new(),
        }
    }
}

/// Direct device message contents.
///
/// Represented as a map of `{ user-ids => { device-ids => message-content } }`.
pub type DirectDeviceMessages =
    BTreeMap<OwnedUserId, BTreeMap<DeviceIdOrAllDevices, RawJson<AnyToDeviceEventContent>>>;
