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

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::push::{
    Action, NewConditionalPushRule, NewPatternedPushRule, NewSimplePushRule, PushCondition,
};
use crate::serde::StringEnum;
use crate::{OwnedRoomId, OwnedUserId, PrivOwnedStr};

/// The kinds of push rules that are available.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RuleKind {
    /// User-configured rules that override all other kinds.
    Override,

    /// Lowest priority user-defined rules.
    Underride,

    /// Sender-specific rules.
    Sender,

    /// Room-specific rules.
    Room,

    /// Content-specific rules.
    Content,

    /// Post-content specific rules.
    #[cfg(feature = "unstable-msc4306")]
    #[palpo_enum(rename = "io.element.msc4306.postcontent")]
    PostContent,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

/// A push rule to update or create.
#[derive(ToSchema, Deserialize, Clone, Debug)]
pub enum NewPushRule {
    /// Rules that override all other kinds.
    Override(NewConditionalPushRule),

    /// Content-specific rules.
    Content(NewPatternedPushRule),

    /// Post-content specific rules.
    #[cfg(feature = "unstable-msc4306")]
    PostContent(NewConditionalPushRule),

    /// Room-specific rules.
    Room(NewSimplePushRule<OwnedRoomId>),

    /// Sender-specific rules.
    Sender(NewSimplePushRule<OwnedUserId>),

    /// Lowest priority rules.
    Underride(NewConditionalPushRule),
}

impl NewPushRule {
    /// The kind of this `NewPushRule`.
    pub fn kind(&self) -> RuleKind {
        match self {
            NewPushRule::Override(_) => RuleKind::Override,
            NewPushRule::Content(_) => RuleKind::Content,
            #[cfg(feature = "unstable-msc4306")]
            NewPushRule::PostContent(_) => RuleKind::PostContent,
            NewPushRule::Room(_) => RuleKind::Room,
            NewPushRule::Sender(_) => RuleKind::Sender,
            NewPushRule::Underride(_) => RuleKind::Underride,
        }
    }

    /// The ID of this `NewPushRule`.
    pub fn rule_id(&self) -> &str {
        match self {
            NewPushRule::Override(r) => &r.rule_id,
            NewPushRule::Content(r) => &r.rule_id,
            #[cfg(feature = "unstable-msc4306")]
            NewPushRule::PostContent(r) => &r.rule_id,
            NewPushRule::Room(r) => r.rule_id.as_ref(),
            NewPushRule::Sender(r) => r.rule_id.as_ref(),
            NewPushRule::Underride(r) => &r.rule_id,
        }
    }
}

/// The scope of a push rule.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "lowercase")]
#[non_exhaustive]
pub enum RuleScope {
    /// The global rules.
    Global,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// Like `SimplePushRule`, but may represent any kind of push rule thanks to
/// `pattern` and `conditions` being optional.
///
/// To create an instance of this type, use one of its `From` implementations.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct PushRule {
    /// The actions to perform when this rule is matched.
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
    /// A rule with no conditions always matches. Only applicable to underride
    /// and override rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<PushCondition>>,

    /// The glob-style pattern to match against.
    ///
    /// Only applicable to content rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct ScopeKindRuleReqArgs {
    /// The scope to fetch rules from.
    #[salvo(parameter(parameter_in = Path))]
    pub scope: RuleScope,

    /// The kind of rule.
    #[salvo(parameter(parameter_in = Path))]
    pub kind: RuleKind,

    /// The identifier for the rule.
    #[salvo(parameter(parameter_in = Path))]
    pub rule_id: String,
}
