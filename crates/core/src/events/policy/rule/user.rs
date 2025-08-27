//! Types for the [`m.policy.rule.user`] event.
//!
//! [`m.policy.rule.user`]: https://spec.matrix.org/latest/client-server-api/#mpolicyruleuser

use crate::macros::EventContent;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use super::{PolicyRuleEventContent, PossiblyRedactedPolicyRuleEventContent};
use crate::events::{PossiblyRedactedStateEventContent, StateEventType, StaticEventContent};

/// The content of an `m.policy.rule.user` event.
///
/// This event type is used to apply rules to user entities.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[allow(clippy::exhaustive_structs)]
#[palpo_event(type = "m.policy.rule.user", kind = State, state_key_type = String, custom_possibly_redacted)]
pub struct PolicyRuleUserEventContent(pub PolicyRuleEventContent);

/// The possibly redacted form of [`PolicyRuleUserEventContent`].
///
/// This type is used when it's not obvious whether the content is redacted or
/// not.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[allow(clippy::exhaustive_structs)]
pub struct PossiblyRedactedPolicyRuleUserEventContent(pub PossiblyRedactedPolicyRuleEventContent);

impl PossiblyRedactedStateEventContent for PossiblyRedactedPolicyRuleUserEventContent {
    type StateKey = String;

    fn event_type(&self) -> StateEventType {
        StateEventType::PolicyRuleUser
    }
}

impl StaticEventContent for PossiblyRedactedPolicyRuleUserEventContent {
    const TYPE: &'static str = PolicyRuleUserEventContent::TYPE;
    type IsPrefix = <PolicyRuleUserEventContent as StaticEventContent>::IsPrefix;
}
