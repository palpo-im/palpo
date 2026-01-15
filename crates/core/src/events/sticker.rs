//! Types for the [`m.sticker`] event.
//!
//! [`m.sticker`]: https://spec.matrix.org/latest/client-server-api/#msticker

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::OwnedMxcUri;
use crate::events::room::ImageInfo;
use crate::macros::EventContent;

/// The content of an `m.sticker` event.
///
/// A sticker message.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.sticker", kind = MessageLike)]
pub struct StickerEventContent {
    /// A textual representation or associated description of the sticker image.
    ///
    /// This could be the alt text of the original image, or a message to
    /// accompany and further describe the sticker.
    pub body: String,

    /// Metadata about the image referred to in `url` including a thumbnail
    /// representation.
    pub info: ImageInfo,

    /// The URL to the sticker image.
    pub url: OwnedMxcUri,
}

impl StickerEventContent {
    /// Creates a new `StickerEventContent` with the given body, image info and
    /// URL.
    pub fn new(body: String, info: ImageInfo, url: OwnedMxcUri) -> Self {
        Self { body, info, url }
    }
}
