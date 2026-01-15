use std::error::Error as StdError;
use std::fmt;

use thiserror::Error;

/// The error type returned when trying to insert a user-defined push rule into
/// a `Ruleset`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InsertPushRuleError {
    /// The rule ID starts with a dot (`.`), which is reserved for
    /// server-default rules.
    #[error("rule IDs starting with a dot are reserved for server-default rules")]
    ServerDefaultRuleId,

    /// The rule ID contains an invalid character.
    #[error("invalid rule ID")]
    InvalidRuleId,

    /// The rule is being placed relative to a server-default rule, which is
    /// forbidden.
    #[error("can't place rule relative to server-default rule")]
    RelativeToServerDefaultRule,

    /// The `before` or `after` rule could not be found.
    #[error("The before or after rule could not be found")]
    UnknownRuleId,

    /// `before` has a higher priority than `after`.
    #[error("before has a higher priority than after")]
    BeforeHigherThanAfter,
}

/// The error type returned when trying modify a push rule that could not be
/// found in a `Ruleset`.
#[derive(Debug, Error)]
#[non_exhaustive]
#[error("The rule could not be found")]
pub struct RuleNotFoundError;

/// The error type returned when trying to remove a user-defined push rule from
/// a `Ruleset`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RemovePushRuleError {
    /// The rule is a server-default rules and they can't be removed.
    #[error("server-default rules cannot be removed")]
    ServerDefault,

    /// The rule was not found.
    #[error("rule not found")]
    NotFound,
}

/// An error that happens when `PushRule` cannot
/// be converted into `PatternedPushRule`
#[derive(Debug)]
pub struct MissingPatternError;

impl fmt::Display for MissingPatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Push rule does not have a pattern.")
    }
}

impl StdError for MissingPatternError {}
