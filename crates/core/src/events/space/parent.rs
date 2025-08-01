//! Types for the [`m.space.parent`] event.
//!
//! [`m.space.parent`]: https://spec.matrix.org/latest/client-server-api/#mspaceparent

use crate::macros::EventContent;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::{OwnedRoomId, OwnedServerName};

/// The content of an `m.space.parent` event.
///
/// Rooms can claim parents via the `m.space.parent` state event.
///
/// Similar to `m.space.child`, the `state_key` is the ID of the parent space,
/// and the content must contain a `via` key which gives a list of candidate
/// servers that can be used to join the parent.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, EventContent)]
#[palpo_event(type = "m.space.parent", kind = State, state_key_type = OwnedRoomId)]
pub struct SpaceParentEventContent {
    /// List of candidate servers that can be used to join the room.
    pub via: Vec<OwnedServerName>,

    /// Determines whether this is the main parent for the space.
    ///
    /// When a user joins a room with a canonical parent, clients may switch to
    /// view the room in the context of that space, peeking into it in order
    /// to find other rooms and group them together. In practice, well
    /// behaved rooms should only have one `canonical` parent, but
    /// given this is not enforced: if multiple are present the client should
    /// select the one with the lowest room ID, as determined via a
    /// lexicographic ordering of the Unicode code-points.
    ///
    /// Defaults to `false`.
    #[serde(default, skip_serializing_if = "palpo_core::serde::is_default")]
    pub canonical: bool,
}

impl SpaceParentEventContent {
    /// Creates a new `SpaceParentEventContent` with the given routing servers.
    pub fn new(via: Vec<OwnedServerName>) -> Self {
        Self {
            via,
            canonical: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, to_value as to_json_value};

    use super::SpaceParentEventContent;
    use crate::server_name;

    #[test]
    fn space_parent_serialization() {
        let content = SpaceParentEventContent {
            via: vec![server_name!("example.com").to_owned()],
            canonical: true,
        };

        let json = json!({
            "via": ["example.com"],
            "canonical": true,
        });

        assert_eq!(to_json_value(&content).unwrap(), json);
    }

    #[test]
    fn space_parent_empty_serialization() {
        let content = SpaceParentEventContent {
            via: vec![],
            canonical: false,
        };

        let json = json!({ "via": [] });

        assert_eq!(to_json_value(&content).unwrap(), json);
    }
}
