//! Types for voice message events ([MSC3245]).
//!
//! [MSC3245]: https://github.com/matrix-org/matrix-spec-proposals/pull/3245

use std::time::Duration;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::events::audio::Amplitude;
use crate::events::file::FileContentBlock;
use crate::events::message::TextContentBlock;
use crate::events::room::message::Relation;
use crate::macros::EventContent;

/// The payload for an extensible voice message.
///
/// This is the new primary type introduced in [MSC3245] and can be sent in
/// rooms with a version that doesn't support extensible events. See the
/// documentation of the [`message`] module for more information.
///
/// [MSC3245]: https://github.com/matrix-org/matrix-spec-proposals/pull/3245
/// [`message`]: super::message
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, EventContent)]
#[palpo_event(type = "org.matrix.msc3245.voice.v2", kind = MessageLike, without_relation)]
pub struct VoiceEventContent {
    /// The text representation of the message.
    #[serde(rename = "org.matrix.msc1767.text")]
    pub text: TextContentBlock,

    /// The file content of the message.
    #[serde(rename = "org.matrix.msc1767.file")]
    pub file: FileContentBlock,

    /// The audio content of the message.
    #[serde(rename = "org.matrix.msc1767.audio_details")]
    pub audio_details: VoiceAudioDetailsContentBlock,

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
    pub relates_to: Option<Relation<VoiceEventContentWithoutRelation>>,
}

impl VoiceEventContent {
    /// Creates a new `VoiceEventContent` with the given fallback
    /// representation, file and audio details.
    pub fn new(
        text: TextContentBlock,
        file: FileContentBlock,
        audio_details: VoiceAudioDetailsContentBlock,
    ) -> Self {
        Self {
            text,
            file,
            audio_details,
            automated: false,
            relates_to: None,
        }
    }

    /// Creates a new `VoiceEventContent` with the given plain text fallback
    /// representation, file and audio details.
    pub fn with_plain_text(
        plain_text: impl Into<String>,
        file: FileContentBlock,
        audio_details: VoiceAudioDetailsContentBlock,
    ) -> Self {
        Self {
            text: TextContentBlock::plain(plain_text),
            file,
            audio_details,
            automated: false,
            relates_to: None,
        }
    }
}

/// A block for details of voice audio content.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct VoiceAudioDetailsContentBlock {
    /// The duration of the audio in seconds.
    #[serde(with = "palpo_core::serde::duration::secs")]
    pub duration: Duration,

    /// The waveform representation of the content.
    #[serde(rename = "org.matrix.msc3246.waveform")]
    pub waveform: Vec<Amplitude>,
}

impl VoiceAudioDetailsContentBlock {
    /// Creates a new `AudioDetailsContentBlock` with the given duration and
    /// waveform representation.
    pub fn new(duration: Duration, waveform: Vec<Amplitude>) -> Self {
        Self { duration, waveform }
    }
}
