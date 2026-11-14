use palpo_core::events::Mentions;
use palpo_core::events::room::message::RoomMessageEventContent;
use serde_json::json;

#[test]
fn mentions_skip_invalid_items_in_streamed_message_content() {
    // Exercise a streaming decoder, including nested non-string values and a
    // valid ID after them, rather than only serde_json::from_value.
    let raw = json!({"msgtype":"m.text", "body":"Hello", "m.mentions": {
        "user_ids": ["@alice:example.org", "not_a_user_id", "@invalid:~",
            {"nested":[{"bad":true}]}, ["@fake:example.org"], null, 5,
            "@bob:example.org", "@alice:example.org"], "room": true
    }})
    .to_string();
    let content: RoomMessageEventContent = serde_json::from_str(&raw).unwrap();
    let mentions = content.mentions.unwrap();
    assert!(mentions.room);
    assert_eq!(
        mentions
            .user_ids
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>(),
        ["@alice:example.org", "@bob:example.org"]
    );
    assert_eq!(
        serde_json::to_value(mentions).unwrap(),
        json!({
            "user_ids":["@alice:example.org", "@bob:example.org"], "room":true
        })
    );
}

#[test]
fn empty_and_all_invalid_mentions_preserve_room_flag() {
    for value in [
        json!({}),
        json!({"user_ids":[]}),
        json!({"user_ids":["bad", {"deep":[1,2]}]}),
    ] {
        let mentions: Mentions = serde_json::from_value(value).unwrap();
        assert!(mentions.user_ids.is_empty());
        assert!(!mentions.room);
    }
    let mentions: Mentions = serde_json::from_value(json!({"room":true})).unwrap();
    assert!(mentions.room);
}

#[test]
fn mention_containers_and_room_flag_remain_strict() {
    for value in [
        json!({"user_ids":null}),
        json!({"user_ids":"@alice:example.org"}),
        json!({"user_ids":{}}),
        json!({"room":"yes"}),
    ] {
        assert!(serde_json::from_value::<Mentions>(value).is_err());
    }
}
