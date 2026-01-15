//! Types for extensible audio message events ([MSC3927]).
//!
//! [MSC3927]: https://github.com/matrix-org/matrix-spec-proposals/pull/3927

use std::time::Duration;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;

mod amplitude_serde;

use super::file::{CaptionContentBlock, FileContentBlock};
use super::message::TextContentBlock;
use super::room::message::Relation;

/// The payload for an extensible audio message.
///
/// This is the new primary type introduced in [MSC3927] and should only be sent
/// in rooms with a version that supports it. See the d ocumentation of the
/// [`message`] module for more information.
///
/// [MSC3927]: https://github.com/matrix-org/matrix-spec-proposals/pull/3927
/// [`message`]: super::message
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, EventContent)]
#[palpo_event(type = "org.matrix.msc1767.audio", kind = MessageLike, without_relation)]
pub struct AudioEventContent {
    /// The text representations of the message.
    #[serde(rename = "org.matrix.msc1767.text")]
    pub text: TextContentBlock,

    /// The file content of the message.
    #[serde(rename = "org.matrix.msc1767.file")]
    pub file: FileContentBlock,

    /// The audio details of the message, if any.
    #[serde(
        rename = "org.matrix.msc1767.audio_details",
        skip_serializing_if = "Option::is_none"
    )]
    pub audio_details: Option<AudioDetailsContentBlock>,

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
    pub relates_to: Option<Relation<AudioEventContentWithoutRelation>>,
}

impl AudioEventContent {
    /// Creates a new `AudioEventContent` with the given text fallback and file.
    pub fn new(text: TextContentBlock, file: FileContentBlock) -> Self {
        Self {
            text,
            file,
            audio_details: None,
            caption: None,
            automated: false,
            relates_to: None,
        }
    }

    /// Creates a new `AudioEventContent` with the given plain text fallback
    /// representation and file.
    pub fn with_plain_text(plain_text: impl Into<String>, file: FileContentBlock) -> Self {
        Self {
            text: TextContentBlock::plain(plain_text),
            file,
            audio_details: None,
            caption: None,
            automated: false,
            relates_to: None,
        }
    }
}

/// A block for details of audio content.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct AudioDetailsContentBlock {
    /// The duration of the audio in seconds.
    #[serde(with = "palpo_core::serde::duration::secs")]
    pub duration: Duration,

    /// The waveform representation of the audio content, if any.
    ///
    /// This is optional and defaults to an empty array.
    #[serde(
        rename = "org.matrix.msc3246.waveform",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub waveform: Vec<Amplitude>,
}

impl AudioDetailsContentBlock {
    /// Creates a new `AudioDetailsContentBlock` with the given duration.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            waveform: Default::default(),
        }
    }
}

/// The amplitude of a waveform sample.
///
/// Must be an integer between 0 and 256.
#[derive(
    ToSchema, Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize,
)]
pub struct Amplitude(u64);

impl Amplitude {
    /// The smallest value that can be represented by this type, 0.
    pub const MIN: u16 = 0;

    /// The largest value that can be represented by this type, 256.
    pub const MAX: u16 = 256;

    /// Creates a new `Amplitude` with the given value.
    ///
    /// It will saturate if it is bigger than [`Amplitude::MAX`].
    pub fn new(value: u16) -> Self {
        Self(value.min(Self::MAX).into())
    }

    /// The value of this `Amplitude`.
    pub fn get(&self) -> u64 {
        self.0
    }
}

impl From<u16> for Amplitude {
    fn from(value: u16) -> Self {
        Self::new(value)
    }
}
