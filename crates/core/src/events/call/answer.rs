//! Types for the [`m.call.answer`] event.
//!
//! [`m.call.answer`]: https://spec.matrix.org/latest/client-server-api/#mcallanswer

use crate::macros::EventContent;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "unstable-msc2747")]
use super::CallCapabilities;
use super::SessionDescription;
use crate::{OwnedVoipId, VoipVersionId};

/// The content of an `m.call.answer` event.
///
/// This event is sent by the callee when they wish to answer the call.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.call.answer", kind = MessageLike)]
pub struct CallAnswerEventContent {
    /// The VoIP session description object.
    pub answer: SessionDescription,

    /// A unique identifier for the call.
    pub call_id: OwnedVoipId,

    /// **Required in VoIP version 1.** A unique ID for this session for the
    /// duration of the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party_id: Option<OwnedVoipId>,

    /// The version of the VoIP specification this messages adheres to.
    pub version: VoipVersionId,

    #[cfg(feature = "unstable-msc2747")]
    /// The VoIP capabilities of the client.
    #[serde(default, skip_serializing_if = "CallCapabilities::is_default")]
    pub capabilities: CallCapabilities,
}

impl CallAnswerEventContent {
    /// Creates an `CallAnswerEventContent` with the given answer, call ID and
    /// VoIP version.
    pub fn new(answer: SessionDescription, call_id: OwnedVoipId, version: VoipVersionId) -> Self {
        Self {
            answer,
            call_id,
            party_id: None,
            version,
            #[cfg(feature = "unstable-msc2747")]
            capabilities: Default::default(),
        }
    }

    /// Convenience method to create a VoIP version 0 `CallAnswerEventContent`
    /// with all the required fields.
    pub fn version_0(answer: SessionDescription, call_id: OwnedVoipId) -> Self {
        Self::new(answer, call_id, VoipVersionId::V0)
    }

    /// Convenience method to create a VoIP version 1 `CallAnswerEventContent`
    /// with all the required fields.
    pub fn version_1(
        answer: SessionDescription,
        call_id: OwnedVoipId,
        party_id: OwnedVoipId,
    ) -> Self {
        Self {
            answer,
            call_id,
            party_id: Some(party_id),
            version: VoipVersionId::V1,
            #[cfg(feature = "unstable-msc2747")]
            capabilities: Default::default(),
        }
    }
}
