//! Common types for the [push notifications module][push].
//!
//! [push]: https://spec.matrix.org/latest/client-server-api/#push-notifications
//!
//! ## Understanding the types of this module
//!
//! Push rules are grouped in `RuleSet`s, and are grouped in five kinds (for
//! more details about the different kind of rules, see the `Ruleset`
//! documentation, or the specification). These five kinds are, by order of
//! priority:
//!
//! - override rules
//! - content rules
//! - room rules
//! - sender rules
//! - underride rules

use std::hash::{Hash, Hasher};

use indexmap::Equivalent;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "unstable-msc3932")]
use crate::push::PredefinedOverrideRuleId;
#[cfg(feature = "unstable-msc3932")]
use crate::push::RoomVersionFeature;
use crate::push::{Action, FlattenedJson, PushCondition, PushConditionRoomCtx, PushRule};

/// Like `SimplePushRule`, but with an additional `conditions` field.
///
/// Only applicable to underride and override rules.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct ConditionalPushRule {
    /// Actions to determine if and how a notification is delivered for events
    /// matching this rule.
    pub actions: Vec<Action>,

    /// Whether this is a default rule, or has been set explicitly.
    pub default: bool,

    /// Whether the push rule is enabled or not.
    pub enabled: bool,

    /// The ID of this rule.
    pub rule_id: String,

    /// The conditions that must hold true for an event in order for a rule to
    /// be applied to an event.
    ///
    /// A rule with no conditions always matches.
    #[serde(default)]
    pub conditions: Vec<PushCondition>,
}

impl ConditionalPushRule {
    /// Check if the push rule applies to the event.
    ///
    /// # Arguments
    ///
    /// * `event` - The flattened JSON representation of a room message event.
    /// * `context` - The context of the room at the time of the event.
    pub async fn applies(&self, event: &FlattenedJson, context: &PushConditionRoomCtx) -> bool {
        if !self.enabled {
            return false;
        }

        // These 3 rules always apply.
        #[cfg(feature = "unstable-msc3932")]
        {
            if self.rule_id != PredefinedOverrideRuleId::Master.as_ref()
                && self.rule_id != PredefinedOverrideRuleId::IsUserMention.as_ref()
                && self.rule_id != PredefinedOverrideRuleId::IsRoomMention.as_ref()
            {
                // Push rules which don't specify a `room_version_supports` condition are
                // assumed to not support extensible events and are therefore
                // expected to be treated as disabled when a room version does
                // support extensible events.
                let room_supports_ext_ev = context
                    .supported_features
                    .contains(&RoomVersionFeature::ExtensibleEvents);
                let rule_has_room_version_supports = self.conditions.iter().any(|condition| {
                    matches!(condition, PushCondition::RoomVersionSupports { .. })
                });

                if room_supports_ext_ev && !rule_has_room_version_supports {
                    return false;
                }
            }
        }

        // The old mention rules (Matrix <1.7) are disabled when m.mentions is present.
        if (self.rule_id == ".m.rule.roomnotif" || self.rule_id == ".m.rule.contains_display_name")
            && event.contains_mentions()
        {
            return false;
        }

        for cond in &self.conditions {
            if !cond.applies(event, context).await {
                return false;
            }
        }
        true
    }
}

// The following trait are needed to be able to make
// an IndexSet of the type

impl Hash for ConditionalPushRule {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.rule_id.hash(state);
    }
}

impl PartialEq for ConditionalPushRule {
    fn eq(&self, other: &Self) -> bool {
        self.rule_id == other.rule_id
    }
}

impl Eq for ConditionalPushRule {}

impl Equivalent<ConditionalPushRule> for str {
    fn equivalent(&self, key: &ConditionalPushRule) -> bool {
        self == key.rule_id
    }
}

/// A conditional push rule to update or create.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct NewConditionalPushRule {
    /// The ID of this rule.
    pub rule_id: String,

    /// The conditions that must hold true for an event in order for a rule to
    /// be applied to an event.
    ///
    /// A rule with no conditions always matches.
    #[serde(default)]
    pub conditions: Vec<PushCondition>,

    /// Actions to determine if and how a notification is delivered for events
    /// matching this rule.
    pub actions: Vec<Action>,
}

impl NewConditionalPushRule {
    /// Creates a `NewConditionalPushRule` with the given ID, conditions and
    /// actions.
    pub fn new(rule_id: String, conditions: Vec<PushCondition>, actions: Vec<Action>) -> Self {
        Self {
            rule_id,
            conditions,
            actions,
        }
    }
}

impl From<ConditionalPushRule> for PushRule {
    fn from(push_rule: ConditionalPushRule) -> Self {
        let ConditionalPushRule {
            actions,
            default,
            enabled,
            rule_id,
            conditions,
            ..
        } = push_rule;
        Self {
            actions,
            default,
            enabled,
            rule_id,
            conditions: Some(conditions),
            pattern: None,
        }
    }
}

impl From<NewConditionalPushRule> for ConditionalPushRule {
    fn from(new_rule: NewConditionalPushRule) -> Self {
        let NewConditionalPushRule {
            rule_id,
            conditions,
            actions,
        } = new_rule;
        Self {
            actions,
            default: false,
            enabled: true,
            rule_id,
            conditions,
        }
    }
}

impl From<PushRule> for ConditionalPushRule {
    fn from(push_rule: PushRule) -> Self {
        let PushRule {
            actions,
            default,
            enabled,
            rule_id,
            conditions,
            ..
        } = push_rule;

        ConditionalPushRule {
            actions,
            default,
            enabled,
            rule_id,
            conditions: conditions.unwrap_or_default(),
        }
    }
}
