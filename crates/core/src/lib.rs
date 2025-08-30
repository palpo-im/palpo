#![allow(missing_docs, dead_code)]

pub mod appservice;
pub mod authentication;
pub mod authorization;
pub mod client;
pub mod device;
pub mod directory;
pub mod encryption;
pub mod events;
pub mod federation;
#[cfg(feature = "html")]
pub mod html;
pub mod identifiers;
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
pub mod version;
pub use version::MatrixVersion;
pub mod error;
pub use error::{MatrixError, UnknownVersionError};
#[macro_use]
pub mod sending;
#[macro_use]
extern crate tracing;
pub mod http_headers;
pub mod media;
pub mod state;
pub mod user;
pub mod utils;

pub use palpo_core_macros as macros;

pub use crate::state::RoomVersion;

// https://github.com/bkchr/proc-macro-crate/issues/10
extern crate self as palpo_core;

use std::fmt;

use ::serde::{Deserialize, Serialize};
use as_variant::as_variant;
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

/// Core types used to define the requests and responses for each endpoint in
/// the various [Matrix API specifications][apis].
///
/// [apis]: https://spec.matrix.org/latest/#matrix-apis
/// An enum to control whether an access token should be added to outgoing
/// requests
#[derive(Clone, Copy, Debug)]
#[allow(clippy::exhaustive_enums)]
pub enum SendAccessToken<'a> {
    /// Add the given access token to the request only if the `METADATA` on the
    /// request requires it.
    IfRequired(&'a str),

    /// Always add the access token.
    Always(&'a str),

    /// Don't add an access token.
    ///
    /// This will lead to an error if the request endpoint requires
    /// authentication
    None,
}

impl<'a> SendAccessToken<'a> {
    /// Get the access token for an endpoint that requires one.
    ///
    /// Returns `Some(_)` if `self` contains an access token.
    pub fn get_required_for_endpoint(self) -> Option<&'a str> {
        as_variant!(self, Self::IfRequired | Self::Always)
    }

    /// Get the access token for an endpoint that should not require one.
    ///
    /// Returns `Some(_)` only if `self` is `SendAccessToken::Always(_)`.
    pub fn get_not_required_for_endpoint(self) -> Option<&'a str> {
        as_variant!(self, Self::Always)
    }
}

/// Authentication scheme used by the endpoint.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(clippy::exhaustive_enums)]
pub enum AuthScheme {
    /// No authentication is performed.
    None,

    /// Authentication is performed by including an access token in the
    /// `Authentication` http header, or an `access_token` query parameter.
    ///
    /// It is recommended to use the header over the query parameter.
    AccessToken,

    /// Authentication is performed by including X-Matrix signatures in the
    /// request headers, as defined in the federation API.
    ServerSignatures,
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
    pub use crate::macros;
    pub use bytes;
    pub use http;
    pub use serde;
    pub use serde_json;
}
