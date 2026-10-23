use palpo_core::events::room::member::RoomMemberEventContent;
use palpo_core::events::room::name::RoomNameEventContent;
use palpo_core::events::{OriginalStateEvent, OriginalSyncStateEvent};
use palpo_core::serde::CanBeEmpty;
use serde_json::{Value, json};

fn state_event(event_type: &str, content: Value, unsigned: Value) -> Value {
    json!({
        "type": event_type,
        "content": content,
        "event_id": "$new:example.org",
        "origin_server_ts": 1,
        "room_id": "!room:example.org",
        "sender": "@alice:example.org",
        "state_key": "",
        "unsigned": unsigned,
    })
}

#[test]
fn generic_state_preserves_replaces_state_in_full_and_sync_events() {
    let event = state_event(
        "m.room.name",
        json!({"name": "New name"}),
        json!({"replaces_state": "$old:example.org"}),
    );
    let full: OriginalStateEvent<RoomNameEventContent> =
        serde_json::from_value(event.clone()).unwrap();
    assert_eq!(
        full.unsigned
            .replaces_state
            .as_deref()
            .map(|id| id.as_str()),
        Some("$old:example.org")
    );
    assert!(full.unsigned.prev_content.is_none());
    assert!(!full.unsigned.is_empty());

    let mut event = event;
    event.as_object_mut().unwrap().remove("room_id");
    let sync: OriginalSyncStateEvent<RoomNameEventContent> = serde_json::from_value(event).unwrap();
    assert_eq!(sync.unsigned.replaces_state, full.unsigned.replaces_state);
    assert!(!sync.unsigned.is_empty());
}

#[test]
fn membership_preserves_replaces_state_in_full_and_sync_events() {
    let mut event = state_event(
        "m.room.member",
        json!({"membership": "join"}),
        json!({"replaces_state": "$old:example.org"}),
    );
    event["state_key"] = json!("@alice:example.org");
    let full: OriginalStateEvent<RoomMemberEventContent> =
        serde_json::from_value(event.clone()).unwrap();
    assert_eq!(
        full.unsigned
            .replaces_state
            .as_deref()
            .map(|id| id.as_str()),
        Some("$old:example.org")
    );
    assert!(!full.unsigned.is_empty());

    event.as_object_mut().unwrap().remove("room_id");
    let sync: OriginalSyncStateEvent<RoomMemberEventContent> =
        serde_json::from_value(event).unwrap();
    assert_eq!(sync.unsigned.replaces_state, full.unsigned.replaces_state);
    assert!(!sync.unsigned.is_empty());
}

#[test]
fn knock_room_state_alone_is_not_empty() {
    let mut event = state_event(
        "m.room.member",
        json!({"membership": "knock"}),
        json!({"knock_room_state": [{
            "type": "m.room.name",
            "state_key": "",
            "sender": "@alice:example.org",
            "content": {"name": "Room name"},
        }]}),
    );
    event["state_key"] = json!("@alice:example.org");
    let full: OriginalStateEvent<RoomMemberEventContent> = serde_json::from_value(event).unwrap();
    assert_eq!(full.unsigned.knock_room_state.len(), 1);
    assert!(!full.unsigned.is_empty());
}

#[test]
fn empty_or_missing_unsigned_remains_empty() {
    for unsigned in [Some(json!({})), None] {
        let mut event = state_event("m.room.name", json!({"name": "Room"}), json!({}));
        if unsigned.is_none() {
            event.as_object_mut().unwrap().remove("unsigned");
        }
        let full: OriginalStateEvent<RoomNameEventContent> =
            serde_json::from_value(event.clone()).unwrap();
        assert!(full.unsigned.is_empty());
        assert!(full.unsigned.replaces_state.is_none());

        event["type"] = json!("m.room.member");
        event["state_key"] = json!("@alice:example.org");
        event["content"] = json!({"membership": "join"});
        let member: OriginalStateEvent<RoomMemberEventContent> =
            serde_json::from_value(event).unwrap();
        assert!(member.unsigned.is_empty());
        assert!(member.unsigned.replaces_state.is_none());
    }
}

#[cfg(feature = "unstable-msc4354")]
#[test]
fn sticky_duration_alone_is_not_empty() {
    let event = state_event(
        "m.room.name",
        json!({"name": "Room"}),
        json!({"msc4354_sticky_duration_ttl_ms": 1000}),
    );
    let full: OriginalStateEvent<RoomNameEventContent> = serde_json::from_value(event).unwrap();
    assert!(!full.unsigned.is_empty());
    assert_eq!(
        full.unsigned.sticky_duration_ttl_ms.unwrap().as_millis(),
        1000
    );
}
