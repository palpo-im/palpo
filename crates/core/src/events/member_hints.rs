//! Types for Matrix member hint state events ([MSC4171]).
//!
//! This implements `m.member_hints` state event described in [MSC4171].
//!
//! [MSC4171]: https://github.com/matrix-org/matrix-spec-proposals/pull/4171

use std::collections::BTreeSet;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::OwnedUserId;
use crate::events::state_key::EmptyStateKey;
use crate::macros::EventContent;

/// The content for an `m.member_hints` state event.
///
/// Any users (service members) listed in the content should not be considered
/// when computing the room name or avatar based on the member list.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, EventContent, PartialEq)]
#[palpo_event(type = "io.element.functional_members", kind = State, state_key_type = EmptyStateKey)]
pub struct MemberHintsEventContent {
    /// The list of user IDs that should be considered a service member of the
    /// room.
    pub service_members: BTreeSet<OwnedUserId>,
}

impl MemberHintsEventContent {
    /// Create a new [`MemberHintsEventContent`] with the given set of service
    /// members.
    pub fn new(service_members: BTreeSet<OwnedUserId>) -> Self {
        Self { service_members }
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeSet;

    use assert_matches2::assert_matches;
    use serde_json::{from_value as from_json_value, json};

    use super::*;
    use crate::events::AnyStateEvent;
    use crate::user_id;

    #[test]
    fn deserialize() {
        let user_id = user_id!("@slackbot:matrix.org");

        let data = json!({
            "type": "io.element.functional_members",
            "state_key": "",
            "content": {
                "service_members": [
                    user_id,
                ]
            },
            "origin_server_ts": 111,
            "event_id": "$3qfxjGYSu4sL25FtR0ep6vePOc",
            "room_id": "!1234:example.org",
            "sender": "@user:example.org"
        });

        let event = from_json_value::<AnyStateEvent>(data)
            .expect("We should be able to deserialize the member hints event");

        assert_matches!(event, AnyStateEvent::MemberHints(event));
        assert_matches!(event, crate::events::StateEvent::Original(event));

        assert!(event.content.service_members.contains(user_id));

        let data = json!({
            "type": "m.member_hints",
            "state_key": "",
            "content": {
                "service_members": [
                    user_id,
                ]
            },
            "origin_server_ts": 111,
            "event_id": "$3qfxjGYSu4sL25FtR0ep6vePOc",
            "room_id": "!1234:example.org",
            "sender": "@user:example.org"
        });

        let event = from_json_value::<AnyStateEvent>(data)
            .expect("We should be able to deserialize the member hints event");

        assert_matches!(event, AnyStateEvent::MemberHints(event));
        assert_matches!(event, crate::events::StateEvent::Original(event));

        assert!(event.content.service_members.contains(user_id));
    }

    #[test]
    fn serialize() {
        let user_id = user_id!("@slackbot:matrix.org");
        let content = MemberHintsEventContent::new(BTreeSet::from([user_id.to_owned()]));

        let serialized = serde_json::to_value(content)
            .expect("We should be able to serialize the member hints content");

        let expected = json!({
            "service_members": [
                user_id,
            ]
        });

        assert_eq!(
            expected, serialized,
            "The serialized member hints content should match the expected one"
        );
    }
}
