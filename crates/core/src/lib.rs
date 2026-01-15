#![allow(missing_docs, dead_code)]

pub mod appservice;
pub mod authentication;
pub mod client;
pub mod device;
pub mod directory;
pub mod encryption;
pub mod events;
pub mod federation;
#[cfg(feature = "html")]
pub mod html;
pub mod identifiers;
pub mod metadata;
mod percent_encode;
pub mod power_levels;
pub mod presence;
pub mod push;
pub mod room;
pub mod room_version_rules;
pub mod serde;
pub mod signatures;
pub mod space;
pub mod third_party;
pub mod third_party_invite;
mod time;
pub mod to_device;
pub use metadata::{MatrixVersion, SupportedVersions};
pub mod error;
pub use error::{MatrixError, UnknownVersionError};
#[macro_use]
pub mod sending;
#[macro_use]
extern crate tracing;
pub mod auth_scheme;
pub mod http_headers;
pub mod media;
// pub mod path_builder;
pub mod state;
pub mod user;
pub mod utils;

pub use palpo_core_macros as macros;

// https://github.com/bkchr/proc-macro-crate/issues/10
extern crate self as palpo_core;

use std::fmt;

use ::serde::{Deserialize, Serialize};
use salvo::oapi::{Components, RefOr, Schema, ToSchema};

pub use self::identifiers::*;
pub use self::time::{UnixMillis, UnixSeconds};

pub type Seqnum = i64;
pub type MatrixResult<T> = Result<T, MatrixError>;

// Wrapper around `Box<str>` that cannot be used in a meaningful way outside of
// this crate:: Used for string enums because their `_Custom` variant can't be
// truly private (only `#[doc(hidden)]`).
#[doc(hidden)]
#[derive(Clone, PartialEq, Eq, PartialOrd, Deserialize, Ord, Hash)]
pub struct PrivOwnedStr(Box<str>);

impl fmt::Debug for PrivOwnedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl ToSchema for PrivOwnedStr {
    fn to_schema(components: &mut Components) -> RefOr<Schema> {
        <String>::to_schema(components)
    }
}

/// The direction to return events from.
#[derive(ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[allow(clippy::exhaustive_enums)]
pub enum Direction {
    /// Return events backwards in time from the requested `from` token.
    #[default]
    #[serde(rename = "b")]
    Backward,

    /// Return events forwards in time from the requested `from` token.
    #[serde(rename = "f")]
    Forward,
}

pub enum ReasonBool<T> {
    True,
    False(T),
}
impl<T> ReasonBool<T> {
    fn value(&self) -> bool {
        matches!(self, Self::True)
    }
}

/// Re-__private used by macro-generated code.
///
/// It is not considered part of this module's public API.
#[doc(hidden)]
pub mod __private {
    pub use {bytes, http, serde, serde_json};

    pub use crate::macros;
}
