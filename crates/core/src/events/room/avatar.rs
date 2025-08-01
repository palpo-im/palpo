//! Types for the [`m.room.avatar`] event.
//!
//! [`m.room.avatar`]: https://spec.matrix.org/latest/client-server-api/#mroomavatar

use crate::macros::EventContent;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use super::ThumbnailInfo;
use crate::{OwnedMxcUri, events::EmptyStateKey};

/// The content of an `m.room.avatar` event.
///
/// A picture that is associated with the room.
///
/// This can be displayed alongside the room information.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[palpo_event(type = "m.room.avatar", kind = State, state_key_type = EmptyStateKey)]
pub struct RoomAvatarEventContent {
    /// Information about the avatar image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<Box<ImageInfo>>,

    /// URL of the avatar image.
    pub url: Option<OwnedMxcUri>,
}

impl RoomAvatarEventContent {
    /// Create an empty `RoomAvatarEventContent`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Metadata about an image (specific to avatars).
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct ImageInfo {
    /// The height of the image in pixels.
    #[serde(rename = "h", skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,

    /// The width of the image in pixels.
    #[serde(rename = "w", skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,

    /// The MIME type of the image, e.g. "image/png."
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mimetype: Option<String>,

    /// The file size of the image in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,

    /// Metadata about the image referred to in `thumbnail_url`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_info: Option<Box<ThumbnailInfo>>,

    /// The URL to the thumbnail of the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<OwnedMxcUri>,

    /// The [BlurHash](https://blurha.sh) for this image.
    ///
    /// This uses the unstable prefix in
    /// [MSC2448](https://github.com/matrix-org/matrix-spec-proposals/pull/2448).
    #[cfg(feature = "unstable-msc2448")]
    #[serde(
        rename = "xyz.amorgan.blurhash",
        skip_serializing_if = "Option::is_none"
    )]
    pub blurhash: Option<String>,
}

impl ImageInfo {
    /// Create a new `ImageInfo` with all fields set to `None`.
    pub fn new() -> Self {
        Self::default()
    }
}
