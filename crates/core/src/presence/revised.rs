//! State resolution and wire compatibility for MSC4532.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PREFIX: &str = "org.continuwuity.presence_v2.msc4532";
pub const PERSISTENT: &str = "org.continuwuity.presence_v2.msc4532.presence.persistent";
pub const STATUS: &str = "org.continuwuity.presence_v2.msc4532.status";
pub const REVISED: &str = "org.continuwuity.presence_v2.msc4532.revised_presence";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum State {
    #[serde(rename = "org.continuwuity.presence_v2.msc4532.active")]
    Active,
    #[serde(rename = "org.continuwuity.presence_v2.msc4532.idle")]
    Idle,
    #[serde(rename = "org.continuwuity.presence_v2.msc4532.busy")]
    Busy,
    #[serde(rename = "offline")]
    Offline,
}

impl State {
    pub fn parse(value: &str, active: bool) -> Option<Self> {
        match value {
            "org.continuwuity.presence_v2.msc4532.active" => Some(Self::Active),
            "org.continuwuity.presence_v2.msc4532.idle" | "unavailable" => Some(Self::Idle),
            "org.continuwuity.presence_v2.msc4532.busy" => Some(Self::Busy),
            "online" => Some(if active { Self::Active } else { Self::Idle }),
            "offline" => Some(Self::Offline),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "org.continuwuity.presence_v2.msc4532.active",
            Self::Idle => "org.continuwuity.presence_v2.msc4532.idle",
            Self::Busy => "org.continuwuity.presence_v2.msc4532.busy",
            Self::Offline => "offline",
        }
    }

    pub fn legacy(self) -> &'static str {
        match self {
            Self::Active => "online",
            Self::Idle | Self::Busy => "unavailable",
            Self::Offline => "offline",
        }
    }
}

#[derive(salvo::oapi::ToSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
    #[serde(flatten)]
    #[salvo(schema(skip))]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub extensions: BTreeMap<String, Value>,
}

pub type Persistent = crate::events::presence::persistent::PresencePersistentEventContent;

/// Disconnected clients cannot be brought online by an override.
pub fn resolve(clients: impl IntoIterator<Item = State>, persistent: &Persistent) -> State {
    let connected: Vec<_> = clients
        .into_iter()
        .filter(|s| *s != State::Offline)
        .collect();
    if connected.is_empty() {
        return State::Offline;
    }
    if let Some(state) = persistent.state_override.as_deref().and_then(|v| {
        // Only the revised enum is valid for overrides; legacy values are unknown.
        serde_json::from_value::<State>(Value::String(v.to_owned())).ok()
    }) {
        return state;
    }
    if connected.contains(&State::Busy) {
        State::Busy
    } else if connected.contains(&State::Active) {
        State::Active
    } else {
        State::Idle
    }
}

/// Both legacy fields and revised fields refer to one authoritative status.
pub fn project(value: &mut Value, state: State, status: &Status, revised: bool) {
    value["presence"] = Value::String(
        if revised {
            state.as_str()
        } else {
            state.legacy()
        }
        .into(),
    );
    value["currently_active"] = Value::Bool(state == State::Active);
    value[STATUS] = serde_json::to_value(status).expect("status is JSON");
    value["status_msg"] = Value::String(status.msg.clone().unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolution_priority_and_unknown_override() {
        let mut p = Persistent::default();
        assert_eq!(resolve([State::Active, State::Busy], &p), State::Busy);
        p.state_override = Some(State::Idle.as_str().into());
        assert_eq!(resolve([State::Active, State::Busy], &p), State::Idle);
        assert_eq!(resolve([State::Offline], &p), State::Offline);
        p.state_override = Some("future".into());
        assert_eq!(resolve([State::Idle, State::Active], &p), State::Active);
    }
    #[test]
    fn status_extensions_and_clear_roundtrip() {
        let p: Persistent =
            serde_json::from_value(serde_json::json!({"status": {"msg":"hi", "example.icon":"x"}}))
                .unwrap();
        let mut wire = serde_json::json!({});
        project(&mut wire, State::Busy, &p.status, false);
        assert_eq!(wire["presence"], "unavailable");
        assert_eq!(wire[STATUS]["example.icon"], "x");
        project(&mut wire, State::Active, &Status::default(), true);
        assert_eq!(wire["status_msg"], "");
        assert_eq!(wire[STATUS], serde_json::json!({}));
    }

    #[test]
    fn federation_and_appservice_status_roundtrip() {
        use crate::appservice::event::EphemeralData;
        use crate::events::presence::PresenceEvent;
        use crate::presence::PresenceUpdate;
        use serde_json::json;

        let status = json!({"msg": "busy", "example.icon": {"name": "moon"}});
        let mut update = json!({
            "user_id": "@alice:example.org",
            "presence": State::Busy.as_str(),
            "status_msg": "busy"
        });
        update[STATUS] = status.clone();
        let parsed: PresenceUpdate = serde_json::from_value(update).unwrap();
        assert_eq!(parsed.presence.as_str(), State::Busy.as_str());
        assert_eq!(parsed.status.as_ref(), Some(&status));
        assert_eq!(serde_json::to_value(parsed).unwrap()[STATUS], status);

        let mut content = json!({});
        project(
            &mut content,
            State::Busy,
            &serde_json::from_value(status.clone()).unwrap(),
            true,
        );
        let event: PresenceEvent = serde_json::from_value(json!({
            "type": "m.presence", "sender": "@alice:example.org", "content": content
        }))
        .unwrap();
        let wire = serde_json::to_value(EphemeralData::Presence(event)).unwrap();
        assert_eq!(wire["type"], "m.presence");
        assert_eq!(wire["content"][STATUS], status);
        assert_eq!(wire["content"]["status_msg"], "busy");
        let parsed: EphemeralData = serde_json::from_value(wire).unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap()["content"][STATUS],
            status
        );

        let legacy: PresenceUpdate = serde_json::from_value(json!({
            "user_id": "@alice:example.org", "presence": "online",
            "currently_active": true, "last_active_ago": 0, "status_msg": "legacy"
        }))
        .unwrap();
        assert!(legacy.status.is_none());
        assert_eq!(
            State::parse(legacy.presence.as_str(), legacy.currently_active),
            Some(State::Active)
        );
        assert_eq!(State::parse("online", false), Some(State::Idle));
        assert_eq!(State::parse("unavailable", false), Some(State::Idle));
    }
}
