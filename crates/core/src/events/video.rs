//! Types for extensible video message events ([MSC3553]).
//!
//! [MSC3553]: https://github.com/matrix-org/matrix-spec-proposals/pull/3553

use std::time::Duration;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use super::file::{CaptionContentBlock, FileContentBlock};
use super::image::ThumbnailContentBlock;
use super::message::TextContentBlock;
use super::room::message::Relation;
use crate::macros::EventContent;

/// The payload for an extensible video message.
///
/// This is the new primary type introduced in [MSC3553] and should only be sent
/// in rooms with a version that supports it. See the documentation of the
/// [`message`] module for more information.
///
/// [MSC3553]: https://github.com/matrix-org/matrix-spec-proposals/pull/3553
/// [`message`]: super::message
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, EventContent)]
#[palpo_event(type = "org.matrix.msc1767.video", kind = MessageLike, without_relation)]
pub struct VideoEventContent {
    /// The text representation of the message.
    #[serde(rename = "org.matrix.msc1767.text")]
    pub text: TextContentBlock,

    /// The file content of the message.
    #[serde(rename = "org.matrix.msc1767.file")]
    pub file: FileContentBlock,

    /// The video details of the message, if any.
    #[serde(
        rename = "org.matrix.msc1767.video_details",
        skip_serializing_if = "Option::is_none"
    )]
    pub video_details: Option<VideoDetailsContentBlock>,

    /// The thumbnails of the message, if any.
    ///
    /// This is optional and defaults to an empty array.
    #[serde(
        rename = "org.matrix.msc1767.thumbnail",
        default,
        skip_serializing_if = "ThumbnailContentBlock::is_empty"
    )]
    pub thumbnail: ThumbnailContentBlock,

    /// The caption of the message, if any.
    #[serde(
        rename = "org.matrix.msc1767.caption",
        skip_serializing_if = "Option::is_none"
    )]
    pub caption: Option<CaptionContentBlock>,

    /// Whether this message is automated.
    #[serde(
        default,
        skip_serializing_if = "palpo_core::serde::is_default",
        rename = "org.matrix.msc1767.automated"
    )]
    pub automated: bool,

    /// Information about related messages.
    #[serde(
        flatten,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::events::room::message::relation_serde::deserialize_relation"
    )]
    pub relates_to: Option<Relation<VideoEventContentWithoutRelation>>,
}

impl VideoEventContent {
    /// Creates a new `VideoEventContent` with the given fallback representation
    /// and file.
    pub fn new(text: TextContentBlock, file: FileContentBlock) -> Self {
        Self {
            text,
            file,
            video_details: None,
            thumbnail: Default::default(),
            caption: None,
            automated: false,
            relates_to: None,
        }
    }

    /// Creates a new `VideoEventContent` with the given plain text fallback
    /// representation and file.
    pub fn with_plain_text(plain_text: impl Into<String>, file: FileContentBlock) -> Self {
        Self {
            text: TextContentBlock::plain(plain_text),
            file,
            video_details: None,
            thumbnail: Default::default(),
            caption: None,
            automated: false,
            relates_to: None,
        }
    }
}

/// A block for details of video content.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct VideoDetailsContentBlock {
    /// The width of the video in pixels.
    pub width: u32,

    /// The height of the video in pixels.
    pub height: u32,

    /// The duration of the video in seconds.
    #[serde(
        with = "palpo_core::serde::duration::opt_secs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub duration: Option<Duration>,
}

impl VideoDetailsContentBlock {
    /// Creates a new `VideoDetailsContentBlock` with the given height and
    /// width.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            duration: None,
        }
    }
}
