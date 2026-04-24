//! Types for the [`m.room.policy`] event.
//!
//! [`m.room.policy`]: https://spec.matrix.org/v1.18/client-server-api/#mroompolicy

use std::collections::BTreeMap;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::events::EmptyStateKey;
use crate::macros::EventContent;
use crate::serde::Base64;
use crate::{OwnedServerName, SigningKeyAlgorithm};

/// The content of an [`m.room.policy`] event.
///
/// A Policy Server configuration.
///
/// If invalid or not set, the room does not use a Policy Server.
///
/// [`m.room.policy`]: https://spec.matrix.org/v1.18/client-server-api/#mroompolicy
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize, EventContent)]
#[palpo_event(type = "m.room.policy", kind = State, state_key_type = EmptyStateKey)]
pub struct RoomPolicyEventContent {
    /// The server name to use as a Policy Server.
    ///
    /// MUST have a joined user in the room.
    pub via: OwnedServerName,

    /// The public keys for the Policy Server.
    ///
    /// MUST contain at least `ed25519`.
    pub public_keys: BTreeMap<SigningKeyAlgorithm, Base64>,
}

impl RoomPolicyEventContent {
    /// Creates a new `RoomPolicyEventContent` with the given server name and ed25519 public key.
    pub fn new(via: OwnedServerName, ed25519_public_key: Base64) -> Self {
        Self {
            via,
            public_keys: [(SigningKeyAlgorithm::Ed25519, ed25519_public_key)].into(),
        }
    }
}
