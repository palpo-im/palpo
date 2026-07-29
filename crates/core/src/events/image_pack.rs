//! Types for [image packs], stabilized from [MSC2545] in Matrix 1.19.
//!
//! [image packs]: https://spec.matrix.org/v1.19/client-server-api/#image-packs
//! [MSC2545]: https://github.com/matrix-org/matrix-spec-proposals/pull/2545

use std::collections::{BTreeMap, BTreeSet};

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::events::room::ImageInfo;
use crate::macros::EventContent;
use crate::serde::{JsonValue, StringEnum};
use crate::{OwnedMxcUri, OwnedRoomId, PrivOwnedStr};

/// The content of an `m.room.image_pack` event.
///
/// The state key is the identifier for the image pack in
/// [ImagePackRoomsEventContent].
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[palpo_event(
    type = "m.room.image_pack",
    kind = State,
    state_key_type = String,
    alias = "m.image_pack",
    alias = "im.ponies.room_emotes"
)]
pub struct RoomImagePackEventContent {
    /// A list of images available in this image pack.
    ///
    /// Keys in the map are shortcodes for the images.
    pub images: BTreeMap<String, ImagePackImage>,

    /// Metadata about the image pack as a whole.
    #[serde(default, skip_serializing_if = "ImagePackMeta::is_empty")]
    pub pack: ImagePackMeta,
}

impl RoomImagePackEventContent {
    /// Creates a new `RoomImagePackEventContent` with a list of images.
    pub fn new(images: BTreeMap<String, ImagePackImage>) -> Self {
        Self {
            images,
            pack: ImagePackMeta::default(),
        }
    }
}

/// The legacy content of an `im.ponies.user_emotes` account data event.
///
/// MSC2545 no longer defines a stable personal image-pack account data event,
/// but this type remains available so existing stored events can still be read.
/// `m.image_pack` is accepted on ingress because it was the personal pack's
/// identifier in an earlier revision of the proposal; it is never serialized.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[palpo_event(
    type = "im.ponies.user_emotes",
    kind = GlobalAccountData,
    alias = "m.image_pack"
)]
pub struct AccountImagePackEventContent {
    /// A list of images available in this image pack.
    ///
    /// Keys in the map are shortcodes for the images.
    pub images: BTreeMap<String, ImagePackImage>,

    /// Image pack metadata.
    #[serde(default, skip_serializing_if = "ImagePackMeta::is_empty")]
    pub pack: ImagePackMeta,
}

impl AccountImagePackEventContent {
    /// Creates a new `AccountImagePackEventContent` with a list of images.
    pub fn new(images: BTreeMap<String, ImagePackImage>) -> Self {
        Self {
            images,
            pack: ImagePackMeta::default(),
        }
    }
}

/// An image object in an image pack.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct ImagePackImage {
    /// The MXC URI to the media file.
    pub url: OwnedMxcUri,

    /// An optional text body for this image.
    /// Useful for the sticker body text or the emote alt text.
    ///
    /// Defaults to the shortcode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,

    /// The [ImageInfo] object used for the `info` block of `m.sticker` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<ImageInfo>,

    /// The usages for this individual image.
    ///
    /// Removed before MSC2545 stabilized: usage is a pack-level field in Matrix 1.19 and this
    /// is empty for spec-conformant packs. It is kept so that a legacy `im.ponies.*` pack that
    /// still carries per-image usages survives a deserialize/serialize round-trip instead of
    /// silently losing whether an image was sticker-only or emoticon-only.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub usage: BTreeSet<PackUsage>,
}

impl ImagePackImage {
    /// Creates a new `ImagePackImage` with the given MXC URI to the media file.
    pub fn new(url: OwnedMxcUri) -> Self {
        Self {
            url,
            body: None,
            info: None,
            usage: BTreeSet::new(),
        }
    }
}

/// Deprecated name for [`ImagePackImage`].
#[deprecated = "use ImagePackImage"]
pub type PackImage = ImagePackImage;

/// Metadata about an image pack.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct ImagePackMeta {
    /// A display name for the pack.
    /// This does not have to be unique from other packs in a room.
    ///
    /// Defaults to the room name, if the image pack event is in the room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// The MXC URI of an avatar/icon to display for the pack.
    ///
    /// Defaults to the room avatar, if the pack is in the room.
    /// Otherwise, the pack does not have an avatar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<OwnedMxcUri>,

    /// The usages for the pack.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub usage: BTreeSet<PackUsage>,

    /// The attribution of this pack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
}

impl ImagePackMeta {
    /// Creates empty image-pack metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether none of the metadata fields are set.
    pub fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.avatar_url.is_none()
            && self.usage.is_empty()
            && self.attribution.is_none()
    }
}

/// Deprecated name for [`ImagePackMeta`].
#[deprecated = "use ImagePackMeta"]
pub type PackInfo = ImagePackMeta;

/// The intended usages for an image pack.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PackUsage {
    /// The pack is usable as a source of emoticons.
    Emoticon,

    /// The pack is usable as a source of stickers.
    Sticker,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// The content of an `m.image_pack.rooms` account data event.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize, EventContent)]
#[palpo_event(
    type = "m.image_pack.rooms",
    kind = GlobalAccountData,
    alias = "im.ponies.emote_rooms"
)]
pub struct ImagePackRoomsEventContent {
    /// A map of room IDs to state keys for globally enabled image packs.
    pub rooms: BTreeMap<OwnedRoomId, BTreeMap<String, RoomImagePackMeta>>,
}

impl ImagePackRoomsEventContent {
    /// Creates a new `ImagePackRoomsEventContent`.
    pub fn new(rooms: BTreeMap<OwnedRoomId, BTreeMap<String, RoomImagePackMeta>>) -> Self {
        Self { rooms }
    }
}

/// Additional metadata for a globally enabled room image pack.
///
/// The spec defines no fields yet and reserves this object for future use. Unrecognised
/// properties are captured so they survive a deserialize/serialize round-trip, as the spec
/// requires them to be preserved when the event is modified.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct RoomImagePackMeta {
    /// Properties not defined by the current spec version.
    #[serde(flatten)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub extra: BTreeMap<String, JsonValue>,
}

impl RoomImagePackMeta {
    /// Creates empty room image-pack metadata.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Deprecated name for [`RoomImagePackMeta`].
#[deprecated = "use RoomImagePackMeta"]
pub type ImagePackRoomContent = RoomImagePackMeta;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::{ImagePackRoomsEventContent, RoomImagePackEventContent};
    use crate::events::{GlobalAccountDataEventType, StateEventType};

    #[test]
    fn room_image_pack_aliases_serialize_to_stable_type() {
        for event_type in ["m.room.image_pack", "m.image_pack", "im.ponies.room_emotes"] {
            assert_eq!(
                StateEventType::from(event_type),
                StateEventType::RoomImagePack
            );
            assert_eq!(
                StateEventType::from(event_type).to_string(),
                "m.room.image_pack"
            );
        }
    }

    #[test]
    fn image_pack_rooms_alias_serializes_to_stable_type() {
        for event_type in ["m.image_pack.rooms", "im.ponies.emote_rooms"] {
            assert_eq!(
                GlobalAccountDataEventType::from(event_type),
                GlobalAccountDataEventType::ImagePackRooms
            );
            assert_eq!(
                GlobalAccountDataEventType::from(event_type).to_string(),
                "m.image_pack.rooms"
            );
        }
    }

    #[test]
    fn empty_pack_metadata_is_omitted() {
        let content = RoomImagePackEventContent::new(BTreeMap::new());

        assert_eq!(to_json_value(content).unwrap(), json!({ "images": {} }));
    }

    #[test]
    fn legacy_personal_pack_remains_readable() {
        assert_eq!(
            GlobalAccountDataEventType::from("im.ponies.user_emotes"),
            GlobalAccountDataEventType::AccountImagePack
        );
    }

    #[test]
    fn legacy_personal_pack_alias_remains_readable() {
        assert_eq!(
            GlobalAccountDataEventType::from("m.image_pack"),
            GlobalAccountDataEventType::AccountImagePack
        );
        assert_eq!(
            GlobalAccountDataEventType::from("m.image_pack").to_string(),
            "im.ponies.user_emotes"
        );
    }

    /// Per-image `usage` predates stabilization, so it must survive a round-trip of a legacy
    /// pack rather than being silently dropped.
    #[test]
    fn legacy_per_image_usage_round_trips() {
        let json = json!({
            "images": {
                "mysticker": {
                    "url": "mxc://example.org/sticker",
                    "usage": ["sticker"]
                }
            }
        });

        let content: RoomImagePackEventContent = from_json_value(json.clone()).unwrap();

        assert_eq!(to_json_value(content).unwrap(), json);
    }

    /// The bottom-level object is reserved for future use and unrecognised properties MUST
    /// survive a modification of the event.
    #[test]
    fn enabled_pack_metadata_preserves_unknown_properties() {
        let json = json!({
            "rooms": {
                "!someroom:example.org": {
                    "": { "com.example.future": { "nested": true } }
                }
            }
        });

        let content: ImagePackRoomsEventContent = from_json_value(json.clone()).unwrap();

        assert_eq!(to_json_value(content).unwrap(), json);
    }
}
