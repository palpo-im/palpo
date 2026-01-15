//! Types for the [`m.identity_server`] event.
//!
//! [`m.identity_server`]: https://spec.matrix.org/latest/client-server-api/#mdirect

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::EventContent;

/// The content of an `m.identity_server` event.
///
/// Persists the user's preferred identity server, or preference to not use an
/// identity server at all.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.identity_server", kind = GlobalAccountData)]
pub struct IdentityServerEventContent {
    /// The URL of the identity server the user prefers to use, or `Null` if the
    /// user does not want to use an identity server.
    ///
    /// If this is `Undefined`, that means the user has not expressed a
    /// preference or has revoked their preference, and any applicable
    /// default should be used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}
