//! A presence event is represented by a struct with a set content field.
//!
//! The only content valid for this event is `PresenceEventContent`.
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::presence::PresenceState;
use crate::{OwnedMxcUri, OwnedUserId};

/// Presence event.
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
#[allow(clippy::exhaustive_structs)]
#[serde(tag = "type", rename = "m.presence")]
pub struct PresenceEvent {
    /// Data specific to the event type.
    pub content: PresenceEventContent,

    /// Contains the fully-qualified ID of the user who sent this event.
    pub sender: OwnedUserId,
}

/// Informs the room of members presence.
///
/// This is the only type a `PresenceEvent` can contain as its `content` field.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct PresenceEventContent {
    /// The current avatar URL for this user.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "palpo_core::serde::empty_string_as_none"
    )]
    pub avatar_url: Option<OwnedMxcUri>,

    /// Whether or not the user is currently active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currently_active: Option<bool>,

    /// The current display name for this user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// The last time since this user performed some action, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_ago: Option<u64>,

    /// The presence state for this user.
    pub presence: PresenceState,

    /// An optional description to accompany the presence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_msg: Option<String>,
}

impl PresenceEventContent {
    /// Creates a new `PresenceEventContent` with the given state.
    pub fn new(presence: PresenceState) -> Self {
        Self {
            avatar_url: None,
            currently_active: None,
            display_name: None,
            last_active_ago: None,
            presence,
            status_msg: None,
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use crate::{mxc_uri, presence::PresenceState};
//     use serde_json::{from_value as from_json_value, json, to_value as
// to_json_value};

//     use super::{PresenceEvent, PresenceEventContent};

//     #[test]
//     fn serialization() {
//         let content = PresenceEventContent {
//             avatar_url:
// Some(mxc_uri!("mxc://localhost/wefuiwegh8742w").to_owned()),
// currently_active: Some(false),             display_name: None,
//             last_active_ago: Some(uint!(2_478_593)),
//             presence: PresenceState::Online,
//             status_msg: Some("Making cupcakes".into()),
//         };

//         let json = json!({
//             "avatar_url": "mxc://localhost/wefuiwegh8742w",
//             "currently_active": false,
//             "last_active_ago": 2_478_593,
//             "presence": "online",
//             "status_msg": "Making cupcakes"
//         });

//         assert_eq!(to_json_value(&content).unwrap(), json);
//     }

//     #[test]
//     fn deserialization() {
//         let json = json!({
//             "content": {
//                 "avatar_url": "mxc://localhost/wefuiwegh8742w",
//                 "currently_active": false,
//                 "last_active_ago": 2_478_593,
//                 "presence": "online",
//                 "status_msg": "Making cupcakes"
//             },
//             "sender": "@example:localhost",
//             "type": "m.presence"
//         });

//         let ev = from_json_value::<PresenceEvent>(json).unwrap();
//         assert_eq!(
//             ev.content.avatar_url.as_deref(),
//             Some(mxc_uri!("mxc://localhost/wefuiwegh8742w"))
//         );
//         assert_eq!(ev.content.currently_active, Some(false));
//         assert_eq!(ev.content.display_name, None);
//         assert_eq!(ev.content.last_active_ago, Some(uint!(2_478_593)));
//         assert_eq!(ev.content.presence, PresenceState::Online);
//         assert_eq!(ev.content.status_msg.as_deref(), Some("Making
// cupcakes"));         assert_eq!(ev.sender, "@example:localhost");

//         let json = json!({
//             "content": {
//                 "avatar_url": "",
//                 "currently_active": false,
//                 "last_active_ago": 2_478_593,
//                 "presence": "online",
//                 "status_msg": "Making cupcakes"
//             },
//             "sender": "@example:localhost",
//             "type": "m.presence"
//         });

//         let ev = from_json_value::<PresenceEvent>(json).unwrap();
//         assert_eq!(ev.content.avatar_url, None);
//         assert_eq!(ev.content.currently_active, Some(false));
//         assert_eq!(ev.content.display_name, None);
//         assert_eq!(ev.content.last_active_ago, Some(uint!(2_478_593)));
//         assert_eq!(ev.content.presence, PresenceState::Online);
//         assert_eq!(ev.content.status_msg.as_deref(), Some("Making
// cupcakes"));         assert_eq!(ev.sender, "@example:localhost");
//     }
// }
