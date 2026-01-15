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

use crate::push::Action;
use crate::push::push_rule::PushRule;

/// A push rule is a single rule that states under what conditions an event
/// should be passed onto a push gateway and how the notification should be
/// presented.
///
/// These rules are stored on the user's homeserver. They are manually
/// configured by the user, who can create and view them via the Client/Server
/// API.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct SimplePushRule<T>
where
    T: 'static,
{
    /// Actions to determine if and how a notification is delivered for events
    /// matching this rule.
    pub actions: Vec<Action>,

    /// Whether this is a default rule, or has been set explicitly.
    pub default: bool,

    /// Whether the push rule is enabled or not.
    pub enabled: bool,

    /// The ID of this rule.
    ///
    /// This is generally the Matrix ID of the entity that it applies to.
    pub rule_id: T,
}

// The following trait are needed to be able to make
// an IndexSet of the type
impl<T> Hash for SimplePushRule<T>
where
    T: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.rule_id.hash(state);
    }
}

impl<T> PartialEq for SimplePushRule<T>
where
    T: PartialEq<T>,
{
    fn eq(&self, other: &Self) -> bool {
        self.rule_id == other.rule_id
    }
}

impl<T> Eq for SimplePushRule<T> where T: Eq {}

impl<T> Equivalent<SimplePushRule<T>> for str
where
    T: AsRef<str>,
{
    fn equivalent(&self, key: &SimplePushRule<T>) -> bool {
        self == key.rule_id.as_ref()
    }
}

/// A simple push rule to update or create.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct NewSimplePushRule<T>
where
    T: ToSchema + 'static,
{
    /// The ID of this rule.
    ///
    /// This is generally the Matrix ID of the entity that it applies to.
    pub rule_id: T,

    /// Actions to determine if and how a notification is delivered for events
    /// matching this rule.
    pub actions: Vec<Action>,
}

impl<T> NewSimplePushRule<T>
where
    T: ToSchema,
{
    /// Creates a `NewSimplePushRule` with the given ID and actions.
    pub fn new(rule_id: T, actions: Vec<Action>) -> Self {
        Self { rule_id, actions }
    }
}

impl<T> From<SimplePushRule<T>> for PushRule
where
    T: Into<String>,
{
    fn from(push_rule: SimplePushRule<T>) -> Self {
        let SimplePushRule {
            actions,
            default,
            enabled,
            rule_id,
            ..
        } = push_rule;
        let rule_id = rule_id.into();
        Self {
            actions,
            default,
            enabled,
            rule_id,
            conditions: None,
            pattern: None,
        }
    }
}
impl<T> From<NewSimplePushRule<T>> for SimplePushRule<T>
where
    T: ToSchema,
{
    fn from(new_rule: NewSimplePushRule<T>) -> Self {
        let NewSimplePushRule { rule_id, actions } = new_rule;
        Self {
            actions,
            default: false,
            enabled: true,
            rule_id,
        }
    }
}
impl<T> TryFrom<PushRule> for SimplePushRule<T>
where
    T: TryFrom<String>,
{
    type Error = <T as TryFrom<String>>::Error;

    fn try_from(push_rule: PushRule) -> Result<Self, Self::Error> {
        let PushRule {
            actions,
            default,
            enabled,
            rule_id,
            ..
        } = push_rule;
        let rule_id = T::try_from(rule_id)?;
        Ok(SimplePushRule {
            actions,
            default,
            enabled,
            rule_id,
        })
    }
}
