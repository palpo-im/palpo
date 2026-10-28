//! Types for MatrixRTC Focus/SFU configurations.

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::PrivOwnedStr;
use crate::macros::StringEnum;

/// Description of the SFU/Focus a membership can be connected to.
///
/// A focus can be any server powering the MatrixRTC session (SFU,
/// MCU). It serves as a node to redistribute RTC streams.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Focus {
    /// LiveKit is one possible type of SFU/Focus that can be used for a MatrixRTC session.
    Livekit(LivekitFocus),
}

/// The struct to describe LiveKit as a `preferred_foci`.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LivekitFocus {
    /// The alias where the LiveKit sessions can be reached.
    ///
    /// Newer Element Call versions omit this legacy field.
    #[serde(rename = "livekit_alias")]
    pub alias: Option<String>,

    /// The URL of the JWT service for the LiveKit instance.
    #[serde(rename = "livekit_service_url")]
    pub service_url: String,
}

impl LivekitFocus {
    /// Initialize a [`LivekitFocus`].
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias with which the LiveKit sessions can be reached.
    /// * `service_url` - The url of the JWT server for the LiveKit instance.
    pub fn new(alias: Option<String>, service_url: String) -> Self {
        Self { alias, service_url }
    }
}

/// Data to define the actively used Focus.
///
/// A focus can be any server powering the MatrixRTC session (SFU,
/// MCU). It serves as a node to redistribute RTC streams.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActiveFocus {
    /// LiveKit is one possible type of SFU/Focus that can be used for a MatrixRTC session.
    Livekit(ActiveLivekitFocus),
}

/// The fields to describe the `active_foci`.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ActiveLivekitFocus {
    /// The selection method used to select the LiveKit focus for the rtc session.
    pub focus_selection: FocusSelection,
}

impl ActiveLivekitFocus {
    /// Initialize a [`ActiveLivekitFocus`].
    ///
    /// # Arguments
    ///
    /// * `focus_selection` - The selection method used to select the LiveKit focus for the rtc
    ///   session.
    pub fn new() -> Self {
        Default::default()
    }
}

/// How to select the active focus for LiveKit
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, Default, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
pub enum FocusSelection {
    /// Select the active focus by using the oldest membership and the oldest focus.
    #[default]
    OldestMembership,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::call::member::CallMemberEventContent;
    use serde_json::json;

    #[test]
    fn rtc_membership_accepts_livekit_with_or_without_alias() {
        for alias in [None, Some("room")] {
            let mut focus =
                json!({"type": "livekit", "livekit_service_url": "https://livekit.example.org"});
            if let Some(alias) = alias {
                focus["livekit_alias"] = json!(alias);
            }
            let content: CallMemberEventContent = serde_json::from_value(json!({
                "application": "m.call", "call_id": "call", "scope": "m.room",
                "device_id": "DEVICE", "expires": 14400000,
                "focus_active": {"type": "livekit", "focus_selection": "oldest_membership"},
                "foci_preferred": [focus]
            }))
            .unwrap();
            let CallMemberEventContent::SessionContent(session) = content else {
                panic!("membership must not fall back to an empty event");
            };
            let Focus::Livekit(focus) = &session.foci_preferred[0];
            assert_eq!(focus.alias.as_deref(), alias);
            let decoded: Focus =
                serde_json::from_value(serde_json::to_value(&session.foci_preferred[0]).unwrap())
                    .unwrap();
            assert_eq!(decoded, session.foci_preferred[0]);
        }
    }
}
