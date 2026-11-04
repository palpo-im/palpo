//! Types for MatrixRTC Focus/SFU configurations.

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::macros::StringEnum;
use crate::{OwnedServerName, PrivOwnedStr};

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
    #[serde(rename = "livekit_alias", skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,

    /// The WebSocket URL of the LiveKit SFU.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// The server that grants access to the LiveKit SFU.
    ///
    /// This is required when the SFU belongs to a different homeserver than the sender's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<OwnedServerName>,

    /// The URL of the legacy JWT service for the LiveKit instance.
    #[serde(
        rename = "livekit_service_url",
        skip_serializing_if = "Option::is_none"
    )]
    #[deprecated = "Use `url` unless compatibility with older MSC4195 deployments is required."]
    pub service_url: Option<String>,
}

impl LivekitFocus {
    /// Initialize a [`LivekitFocus`] with an SFU WebSocket URL.
    ///
    /// # Arguments
    ///
    /// * `url` - The WebSocket URL of the LiveKit SFU.
    #[allow(deprecated)]
    pub fn new(url: String) -> Self {
        Self {
            alias: None,
            url: Some(url),
            server_name: None,
            service_url: None,
        }
    }

    /// Initialize a [`LivekitFocus`] with a legacy JWT service URL.
    #[deprecated = "Use `LivekitFocus::new()` unless compatibility with older MSC4195 deployments is required."]
    #[allow(deprecated)]
    pub fn with_service_url(service_url: String) -> Self {
        Self {
            alias: None,
            url: None,
            server_name: None,
            service_url: Some(service_url),
        }
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
    use serde_json::json;

    use super::*;
    use crate::events::call::member::CallMemberEventContent;

    #[test]
    #[allow(deprecated)]
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
            assert_eq!(focus.url, None);
            assert_eq!(focus.server_name, None);
            assert_eq!(
                focus.service_url.as_deref(),
                Some("https://livekit.example.org")
            );
            let wire = serde_json::to_value(&session.foci_preferred[0]).unwrap();
            assert_eq!(wire.get("livekit_alias").is_some(), alias.is_some());
            let decoded: Focus = serde_json::from_value(wire).unwrap();
            assert_eq!(decoded, session.foci_preferred[0]);
        }
    }

    #[test]
    #[allow(deprecated)]
    fn rtc_membership_roundtrips_current_and_legacy_livekit_fields() {
        let wire = json!({
            "type": "livekit",
            "url": "wss://livekit.example.org",
            "server_name": "other.example.org",
            "livekit_service_url": "https://livekit-jwt.example.org",
            "livekit_alias": "legacy-room"
        });
        let focus: Focus = serde_json::from_value(wire.clone()).unwrap();
        let Focus::Livekit(livekit) = &focus;

        assert_eq!(livekit.alias.as_deref(), Some("legacy-room"));
        assert_eq!(livekit.url.as_deref(), Some("wss://livekit.example.org"));
        assert_eq!(
            livekit.server_name.as_deref(),
            Some(crate::server_name!("other.example.org"))
        );
        assert_eq!(
            livekit.service_url.as_deref(),
            Some("https://livekit-jwt.example.org")
        );
        assert_eq!(serde_json::to_value(focus).unwrap(), wire);
    }
}
