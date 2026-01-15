//! Types for the `m.poll.start` event.

use std::ops::Deref;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;

mod poll_answers_serde;
use poll_answers_serde::PollAnswersDeHelper;

use super::end::{PollEndEventContent, PollResultsContentBlock};
use super::{PollResponseData, compile_poll_results, generate_poll_end_fallback_text};
use crate::events::message::TextContentBlock;
use crate::events::room::message::Relation;
use crate::serde::StringEnum;
use crate::{PrivOwnedStr, UnixMillis};

/// The payload for a poll start event.
///
/// This is the event content that should be sent for room versions that support
/// extensible events. As of Matrix 1.7, none of the stable room versions (1
/// through 10) support extensible events.
///
/// To send a poll start event for a room version that does not support
/// extensible events, use [`UnstablePollStartEventContent`].
///
/// [`UnstablePollStartEventContent`]: super::unstable_start::UnstablePollStartEventContent
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, EventContent)]
#[palpo_event(type = "m.poll.start", kind = MessageLike, without_relation)]
pub struct PollStartEventContent {
    /// The poll content of the message.
    #[serde(rename = "m.poll")]
    pub poll: PollContentBlock,

    /// Text representation of the message, for clients that don't support
    /// polls.
    #[serde(rename = "m.text")]
    pub text: TextContentBlock,

    /// Information about related messages.
    #[serde(
        flatten,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::events::room::message::relation_serde::deserialize_relation"
    )]
    pub relates_to: Option<Relation<PollStartEventContentWithoutRelation>>,

    /// Whether this message is automated.
    #[cfg(feature = "unstable-msc3955")]
    #[serde(
        default,
        skip_serializing_if = "palpo_core::serde::is_default",
        rename = "org.matrix.msc1767.automated"
    )]
    pub automated: bool,
}

impl PollStartEventContent {
    /// Creates a new `PollStartEventContent` with the given fallback
    /// representation and poll content.
    pub fn new(text: TextContentBlock, poll: PollContentBlock) -> Self {
        Self {
            poll,
            text,
            relates_to: None,
            #[cfg(feature = "unstable-msc3955")]
            automated: false,
        }
    }

    /// Creates a new `PollStartEventContent` with the given plain text fallback
    /// representation and poll content.
    pub fn with_plain_text(plain_text: impl Into<String>, poll: PollContentBlock) -> Self {
        Self::new(TextContentBlock::plain(plain_text), poll)
    }
}

impl OriginalSyncPollStartEvent {
    /// Compile the results for this poll with the given response into a
    /// `PollEndEventContent`.
    ///
    /// It generates a default text representation of the results in English.
    ///
    /// This uses [`compile_poll_results()`] internally.
    pub fn compile_results<'a>(
        &'a self,
        responses: impl IntoIterator<Item = PollResponseData<'a>>,
    ) -> PollEndEventContent {
        let full_results =
            compile_poll_results(&self.content.poll, responses, Some(UnixMillis::now()));
        let results = full_results
            .into_iter()
            .map(|(id, users)| (id, users.len()))
            .collect::<Vec<_>>();

        // Construct the results and get the top answer(s).
        let poll_results = PollResultsContentBlock::from_iter(
            results
                .iter()
                .map(|(id, count)| ((*id).to_owned(), (*count).try_into().unwrap_or(u64::MAX))),
        );

        // Get the text representation of the best answers.
        let answers = self
            .content
            .poll
            .answers
            .iter()
            .map(|a| {
                let text = a.text.find_plain().unwrap_or(&a.id);
                (a.id.as_str(), text)
            })
            .collect::<Vec<_>>();
        let plain_text = generate_poll_end_fallback_text(&answers, results.into_iter());

        let mut end = PollEndEventContent::with_plain_text(plain_text, self.event_id.clone());
        end.poll_results = Some(poll_results);

        end
    }
}

/// A block for poll content.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct PollContentBlock {
    /// The question of the poll.
    pub question: PollQuestion,

    /// The kind of the poll.
    #[serde(default, skip_serializing_if = "palpo_core::serde::is_default")]
    pub kind: PollKind,

    /// The maximum number of responses a user is able to select.
    ///
    /// Must be greater or equal to `1`.
    ///
    /// Defaults to `1`.
    #[serde(
        default = "PollContentBlock::default_max_selections",
        skip_serializing_if = "PollContentBlock::max_selections_is_default"
    )]
    pub max_selections: u32,

    /// The possible answers to the poll.
    pub answers: PollAnswers,
}

impl PollContentBlock {
    /// Creates a new `PollStartContent` with the given question and answers.
    pub fn new(question: TextContentBlock, answers: PollAnswers) -> Self {
        Self {
            question: question.into(),
            kind: Default::default(),
            max_selections: Self::default_max_selections(),
            answers,
        }
    }

    pub(super) fn default_max_selections() -> u32 {
        1
    }

    fn max_selections_is_default(max_selections: &u32) -> bool {
        max_selections == &Self::default_max_selections()
    }
}

/// The question of a poll.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct PollQuestion {
    /// The text representation of the question.
    #[serde(rename = "m.text")]
    pub text: TextContentBlock,
}

impl From<TextContentBlock> for PollQuestion {
    fn from(text: TextContentBlock) -> Self {
        Self { text }
    }
}

/// The kind of poll.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, Default, StringEnum)]
pub enum PollKind {
    /// The results are revealed once the poll is closed.
    #[default]
    #[palpo_enum(rename = "m.undisclosed")]
    Undisclosed,

    /// The votes are visible up until and including when the poll is closed.
    #[palpo_enum(rename = "m.disclosed")]
    Disclosed,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

/// The answers to a poll.
///
/// Must include between 1 and 20 `PollAnswer`s.
///
/// To build this, use the `TryFrom` implementations.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[serde(try_from = "PollAnswersDeHelper")]
pub struct PollAnswers(Vec<PollAnswer>);

impl PollAnswers {
    /// The smallest number of values contained in a `PollAnswers`.
    pub const MIN_LENGTH: usize = 1;

    /// The largest number of values contained in a `PollAnswers`.
    pub const MAX_LENGTH: usize = 20;
}

/// An error encountered when trying to convert to a `PollAnswers`.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum PollAnswersError {
    /// There are more than [`PollAnswers::MAX_LENGTH`] values.
    #[error("too many values")]
    TooManyValues,
    /// There are less that [`PollAnswers::MIN_LENGTH`] values.
    #[error("not enough values")]
    NotEnoughValues,
}

impl TryFrom<Vec<PollAnswer>> for PollAnswers {
    type Error = PollAnswersError;

    fn try_from(value: Vec<PollAnswer>) -> Result<Self, Self::Error> {
        if value.len() < Self::MIN_LENGTH {
            Err(PollAnswersError::NotEnoughValues)
        } else if value.len() > Self::MAX_LENGTH {
            Err(PollAnswersError::TooManyValues)
        } else {
            Ok(Self(value))
        }
    }
}

impl TryFrom<&[PollAnswer]> for PollAnswers {
    type Error = PollAnswersError;

    fn try_from(value: &[PollAnswer]) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl Deref for PollAnswers {
    type Target = [PollAnswer];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Poll answer.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct PollAnswer {
    /// The ID of the answer.
    ///
    /// This must be unique among the answers of a poll.
    #[serde(rename = "m.id")]
    pub id: String,

    /// The text representation of the answer.
    #[serde(rename = "m.text")]
    pub text: TextContentBlock,
}

impl PollAnswer {
    /// Creates a new `PollAnswer` with the given id and text representation.
    pub fn new(id: String, text: TextContentBlock) -> Self {
        Self { id, text }
    }
}
