//! `GET /_matrix/client/unstable/io.element.msc4388/rendezvous`
//!
//! Discover if the rendezvous API is available.

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

/// Request type for the `discover_rendezvous` endpoint.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct DiscoverRendezvousReqBody {}

impl DiscoverRendezvousReqBody {
    /// Creates a new `DiscoverRendezvousReqBody`.
    pub fn new() -> Self {
        Self {}
    }
}

/// Response type for the `discover_rendezvous` endpoint.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
pub struct DiscoverRendezvousResBody {
    /// Whether the requester is able to use the create session endpoint.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub create_available: bool,
}

impl DiscoverRendezvousResBody {
    /// Creates a new `DiscoverRendezvousResBody`.
    pub fn new(create_available: bool) -> Self {
        Self { create_available }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::DiscoverRendezvousResBody;

    #[test]
    fn rendezvous_discovery_deserializes_old_empty_payload() {
        let body: DiscoverRendezvousResBody = from_json_value(json!({})).unwrap();

        assert!(!body.create_available);
    }

    #[test]
    fn rendezvous_discovery_serializes_available_flag() {
        assert_eq!(
            to_json_value(DiscoverRendezvousResBody::new(true)).unwrap(),
            json!({ "create_available": true })
        );
    }

    #[test]
    fn rendezvous_discovery_omits_unavailable_flag() {
        assert_eq!(
            to_json_value(DiscoverRendezvousResBody::new(false)).unwrap(),
            json!({})
        );
    }
}
