//! Constructors for [predefined push rules].
//!
//! [predefined push rules]: https://spec.matrix.org/latest/client-server-api/#predefined-rules

use indexmap::IndexSet;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::{
    Action, AnyPushRuleRef, ConditionalPushRule, FlattenedJson, InsertPushRuleError, NewPushRule,
    PatternedPushRule, PushConditionRoomCtx, RuleKind, RuleNotFoundError, RulesetIter,
    SimplePushRule, insert_and_move_rule,
};
use crate::push::RemovePushRuleError;
use crate::serde::RawJson;
use crate::{OwnedRoomId, OwnedUserId};

/// A push ruleset scopes a set of rules according to some criteria.
///
/// For example, some rules may only be applied for messages from a particular
/// sender, a particular room, or by default. The push ruleset contains the
/// entire set of scopes and rules.
#[derive(ToSchema, Deserialize, Serialize, Default, Clone, Debug)]
pub struct Ruleset {
    /// These rules configure behavior for (unencrypted) messages that match
    /// certain patterns.
    #[serde(default, skip_serializing_if = "IndexSet::is_empty")]
    #[salvo(schema(value_type = HashSet<PatternedPushRule>))]
    pub content: IndexSet<PatternedPushRule>,

    /// These rules are identical to override rules, but have a lower priority than `room` and
    /// `sender` rules.
    #[cfg(feature = "unstable-msc4306")]
    #[serde(default, skip_serializing_if = "IndexSet::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub postcontent: IndexSet<ConditionalPushRule>,

    /// These user-configured rules are given the highest priority.
    ///
    /// This field is named `override_` instead of `override` because the latter
    /// is a reserved keyword in Rust.
    #[serde(
        rename = "override",
        default,
        skip_serializing_if = "IndexSet::is_empty"
    )]
    #[salvo(schema(value_type = HashSet<ConditionalPushRule>))]
    pub override_: IndexSet<ConditionalPushRule>,

    /// These rules change the behavior of all messages for a given room.
    #[serde(default, skip_serializing_if = "IndexSet::is_empty")]
    #[salvo(schema(value_type = HashSet<SimplePushRule<OwnedRoomId>>))]
    pub room: IndexSet<SimplePushRule<OwnedRoomId>>,

    /// These rules configure notification behavior for messages from a specific
    /// Matrix user ID.
    #[serde(default, skip_serializing_if = "IndexSet::is_empty")]
    #[salvo(schema(value_type = HashSet<SimplePushRule<OwnedUserId>>))]
    pub sender: IndexSet<SimplePushRule<OwnedUserId>>,

    /// These rules are identical to override rules, but have a lower priority
    /// than `content`, `room` and `sender` rules.
    #[serde(default, skip_serializing_if = "IndexSet::is_empty")]
    #[salvo(schema(value_type = HashSet<ConditionalPushRule>))]
    pub underride: IndexSet<ConditionalPushRule>,
}
impl Ruleset {
    /// Creates an empty `Ruleset`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Creates a borrowing iterator over all push rules in this `Ruleset`.
    ///
    /// For an owning iterator, use `.into_iter()`.
    pub fn iter(&self) -> RulesetIter<'_> {
        self.into_iter()
    }

    /// Inserts a user-defined rule in the rule set.
    ///
    /// If a rule with the same kind and `rule_id` exists, it will be replaced.
    ///
    /// If `after` or `before` is set, the rule will be moved relative to the
    /// rule with the given ID. If both are set, the rule will become the
    /// next-most important rule with respect to `before`. If neither are
    /// set, and the rule is newly inserted, it will become the rule with
    /// the highest priority of its kind.
    ///
    /// Returns an error if the parameters are invalid.
    pub fn insert(
        &mut self,
        rule: NewPushRule,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<(), InsertPushRuleError> {
        let rule_id = rule.rule_id();
        if rule_id.starts_with('.') {
            return Err(InsertPushRuleError::ServerDefaultRuleId);
        }
        if rule_id.contains('/') {
            return Err(InsertPushRuleError::InvalidRuleId);
        }
        if rule_id.contains('\\') {
            return Err(InsertPushRuleError::InvalidRuleId);
        }
        if after.is_some_and(|s| s.starts_with('.')) {
            return Err(InsertPushRuleError::RelativeToServerDefaultRule);
        }
        if before.is_some_and(|s| s.starts_with('.')) {
            return Err(InsertPushRuleError::RelativeToServerDefaultRule);
        }

        match rule {
            NewPushRule::Override(r) => {
                let mut rule = ConditionalPushRule::from(r);

                if let Some(prev_rule) = self.override_.get(rule.rule_id.as_str()) {
                    rule.enabled = prev_rule.enabled;
                }

                // `m.rule.master` should always be the rule with the highest priority, so we
                // insert this one at most at the second place.
                let default_position = 1;

                insert_and_move_rule(&mut self.override_, rule, default_position, after, before)
            }
            #[cfg(feature = "unstable-msc4306")]
            NewPushRule::PostContent(r) => {
                let mut rule = ConditionalPushRule::from(r);

                if let Some(prev_rule) = self.postcontent.get(rule.rule_id.as_str()) {
                    rule.enabled = prev_rule.enabled;
                }

                insert_and_move_rule(&mut self.postcontent, rule, 0, after, before)
            }
            NewPushRule::Underride(r) => {
                let mut rule = ConditionalPushRule::from(r);

                if let Some(prev_rule) = self.underride.get(rule.rule_id.as_str()) {
                    rule.enabled = prev_rule.enabled;
                }

                insert_and_move_rule(&mut self.underride, rule, 0, after, before)
            }
            NewPushRule::Content(r) => {
                let mut rule = PatternedPushRule::from(r);

                if let Some(prev_rule) = self.content.get(rule.rule_id.as_str()) {
                    rule.enabled = prev_rule.enabled;
                }

                insert_and_move_rule(&mut self.content, rule, 0, after, before)
            }
            NewPushRule::Room(r) => {
                let mut rule = SimplePushRule::from(r);

                if let Some(prev_rule) = self.room.get(rule.rule_id.as_str()) {
                    rule.enabled = prev_rule.enabled;
                }

                insert_and_move_rule(&mut self.room, rule, 0, after, before)
            }
            NewPushRule::Sender(r) => {
                let mut rule = SimplePushRule::from(r);

                if let Some(prev_rule) = self.sender.get(rule.rule_id.as_str()) {
                    rule.enabled = prev_rule.enabled;
                }

                insert_and_move_rule(&mut self.sender, rule, 0, after, before)
            }
        }
    }

    /// Get the rule from the given kind and with the given `rule_id` in the
    /// rule set.
    pub fn get(&self, kind: RuleKind, rule_id: impl AsRef<str>) -> Option<AnyPushRuleRef<'_>> {
        let rule_id = rule_id.as_ref();

        match kind {
            RuleKind::Override => self.override_.get(rule_id).map(AnyPushRuleRef::Override),
            RuleKind::Underride => self.underride.get(rule_id).map(AnyPushRuleRef::Underride),
            RuleKind::Sender => self.sender.get(rule_id).map(AnyPushRuleRef::Sender),
            RuleKind::Room => self.room.get(rule_id).map(AnyPushRuleRef::Room),
            RuleKind::Content => self.content.get(rule_id).map(AnyPushRuleRef::Content),
            #[cfg(feature = "unstable-msc4306")]
            RuleKind::PostContent => self
                .postcontent
                .get(rule_id)
                .map(AnyPushRuleRef::PostContent),
            RuleKind::_Custom(_) => None,
        }
    }

    /// Set whether the rule from the given kind and with the given `rule_id` in
    /// the rule set is enabled.
    ///
    /// Returns an error if the rule can't be found.
    pub fn set_enabled(
        &mut self,
        kind: RuleKind,
        rule_id: impl AsRef<str>,
        enabled: bool,
    ) -> Result<(), RuleNotFoundError> {
        let rule_id = rule_id.as_ref();

        match kind {
            RuleKind::Override => {
                let mut rule = self
                    .override_
                    .get(rule_id)
                    .ok_or(RuleNotFoundError)?
                    .clone();
                rule.enabled = enabled;
                self.override_.replace(rule);
            }
            RuleKind::Underride => {
                let mut rule = self
                    .underride
                    .get(rule_id)
                    .ok_or(RuleNotFoundError)?
                    .clone();
                rule.enabled = enabled;
                self.underride.replace(rule);
            }
            RuleKind::Sender => {
                let mut rule = self.sender.get(rule_id).ok_or(RuleNotFoundError)?.clone();
                rule.enabled = enabled;
                self.sender.replace(rule);
            }
            RuleKind::Room => {
                let mut rule = self.room.get(rule_id).ok_or(RuleNotFoundError)?.clone();
                rule.enabled = enabled;
                self.room.replace(rule);
            }
            RuleKind::Content => {
                let mut rule = self.content.get(rule_id).ok_or(RuleNotFoundError)?.clone();
                rule.enabled = enabled;
                self.content.replace(rule);
            }
            #[cfg(feature = "unstable-msc4306")]
            RuleKind::PostContent => {
                let mut rule = self
                    .postcontent
                    .get(rule_id)
                    .ok_or(RuleNotFoundError)?
                    .clone();
                rule.enabled = enabled;
                self.postcontent.replace(rule);
            }
            RuleKind::_Custom(_) => return Err(RuleNotFoundError),
        }

        Ok(())
    }

    /// Set the actions of the rule from the given kind and with the given
    /// `rule_id` in the rule set.
    ///
    /// Returns an error if the rule can't be found.
    pub fn set_actions(
        &mut self,
        kind: RuleKind,
        rule_id: impl AsRef<str>,
        actions: Vec<Action>,
    ) -> Result<(), RuleNotFoundError> {
        let rule_id = rule_id.as_ref();

        match kind {
            RuleKind::Override => {
                let mut rule = self
                    .override_
                    .get(rule_id)
                    .ok_or(RuleNotFoundError)?
                    .clone();
                rule.actions = actions;
                self.override_.replace(rule);
            }
            RuleKind::Underride => {
                let mut rule = self
                    .underride
                    .get(rule_id)
                    .ok_or(RuleNotFoundError)?
                    .clone();
                rule.actions = actions;
                self.underride.replace(rule);
            }
            RuleKind::Sender => {
                let mut rule = self.sender.get(rule_id).ok_or(RuleNotFoundError)?.clone();
                rule.actions = actions;
                self.sender.replace(rule);
            }
            RuleKind::Room => {
                let mut rule = self.room.get(rule_id).ok_or(RuleNotFoundError)?.clone();
                rule.actions = actions;
                self.room.replace(rule);
            }
            RuleKind::Content => {
                let mut rule = self.content.get(rule_id).ok_or(RuleNotFoundError)?.clone();
                rule.actions = actions;
                self.content.replace(rule);
            }
            #[cfg(feature = "unstable-msc4306")]
            RuleKind::PostContent => {
                let mut rule = self
                    .postcontent
                    .get(rule_id)
                    .ok_or(RuleNotFoundError)?
                    .clone();
                rule.actions = actions;
                self.postcontent.replace(rule);
            }
            RuleKind::_Custom(_) => return Err(RuleNotFoundError),
        }

        Ok(())
    }

    /// Get the first push rule that applies to this event, if any.
    ///
    /// # Arguments
    ///
    /// * `event` - The raw JSON of a room message event.
    /// * `context` - The context of the message and room at the time of the event.
    #[instrument(skip_all, fields(context.room_id = %context.room_id))]
    pub async fn get_match<T>(
        &self,
        event: &RawJson<T>,
        context: &PushConditionRoomCtx,
    ) -> Option<AnyPushRuleRef<'_>> {
        let event = FlattenedJson::from_raw(event);

        if event
            .get_str("sender")
            .is_some_and(|sender| sender == context.user_id)
        {
            // no need to look at the rules if the event was by the user themselves
            return None;
        }

        for rule in self {
            if rule.applies(&event, context).await {
                return Some(rule);
            }
        }

        None
    }

    /// Get the push actions that apply to this event.
    ///
    /// Returns an empty slice if no push rule applies.
    ///
    /// # Arguments
    ///
    /// * `event` - The raw JSON of a room message event.
    /// * `context` - The context of the message and room at the time of the event.
    #[instrument(skip_all, fields(context.room_id = %context.room_id))]
    pub async fn get_actions<T>(
        &self,
        event: &RawJson<T>,
        context: &PushConditionRoomCtx,
    ) -> &[Action] {
        self.get_match(event, context)
            .await
            .map(|rule| rule.actions())
            .unwrap_or(&[])
    }

    /// Removes a user-defined rule in the rule set.
    ///
    /// Returns an error if the parameters are invalid.
    pub fn remove(
        &mut self,
        kind: RuleKind,
        rule_id: impl AsRef<str>,
    ) -> Result<(), RemovePushRuleError> {
        let rule_id = rule_id.as_ref();

        if let Some(rule) = self.get(kind.clone(), rule_id) {
            if rule.is_server_default() {
                return Err(RemovePushRuleError::ServerDefault);
            }
        } else {
            return Err(RemovePushRuleError::NotFound);
        }

        match kind {
            RuleKind::Override => {
                self.override_.shift_remove(rule_id);
            }
            RuleKind::Underride => {
                self.underride.shift_remove(rule_id);
            }
            RuleKind::Sender => {
                self.sender.shift_remove(rule_id);
            }
            RuleKind::Room => {
                self.room.shift_remove(rule_id);
            }
            RuleKind::Content => {
                self.content.shift_remove(rule_id);
            }
            #[cfg(feature = "unstable-msc4306")]
            RuleKind::PostContent => {
                self.postcontent.shift_remove(rule_id);
            }
            // This has been handled in the `self.get` call earlier.
            RuleKind::_Custom(_) => unreachable!(),
        }

        Ok(())
    }
}

// #[cfg(test)]
// mod tests {
//     use assert_matches2::assert_matches;
//     use assign::assign;

//     use super::PredefinedOverrideRuleId;
//     use crate::{
//         push::{Action, ConditionalPushRule, ConditionalPushRuleInit,
// Ruleset},         user_id,
//     };

//     #[test]
//     fn update_with_server_default() {
//         let user_rule_id = "user_always_true";
//         let default_rule_id = ".default_always_true";

//         let override_ = [
//             // Default `.m.rule.master` push rule with non-default state.
//             assign!(ConditionalPushRule::master(), { enabled: true, actions:
// vec![Action::Notify]}),             // User-defined push rule.
//             ConditionalPushRuleInit {
//                 actions: vec![],
//                 default: false,
//                 enabled: false,
//                 rule_id: user_rule_id.to_owned(),
//                 conditions: vec![],
//             }
//             .into(),
//             // Old server-default push rule.
//             ConditionalPushRuleInit {
//                 actions: vec![],
//                 default: true,
//                 enabled: true,
//                 rule_id: default_rule_id.to_owned(),
//                 conditions: vec![],
//             }
//             .into(),
//         ]
//         .into_iter()
//         .collect();
//         let mut ruleset = Ruleset {
//             override_,
//             ..Default::default()
//         };

//         let new_server_default =
// Ruleset::server_default(user_id!("@user:localhost"));

//         ruleset.update_with_server_default(new_server_default);

//         // Master rule is in first position.
//         let master_rule = &ruleset.override_[0];
//         assert_eq!(master_rule.rule_id,
// PredefinedOverrideRuleId::Master.as_str());

//         // `enabled` and `actions` have been copied from the old rules.
//         assert!(master_rule.enabled);
//         assert_eq!(master_rule.actions.len(), 1);
//         assert_matches!(&master_rule.actions[0], Action::Notify);

//         // Non-server-default rule is still present and hasn't changed.
//         let user_rule = ruleset.override_.get(user_rule_id).unwrap();
//         assert!(!user_rule.enabled);
//         assert_eq!(user_rule.actions.len(), 0);

//         // Old server-default rule is gone.
//         assert_matches!(ruleset.override_.get(default_rule_id), None);

//         // New server-default rule is present and hasn't changed.
//         let member_event_rule = ruleset
//             .override_
//             .get(PredefinedOverrideRuleId::MemberEvent.as_str())
//             .unwrap();
//         assert!(member_event_rule.enabled);
//         assert_eq!(member_event_rule.actions.len(), 0);
//     }
// }
