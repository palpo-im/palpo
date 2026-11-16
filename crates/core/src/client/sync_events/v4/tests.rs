use serde_json::{Value, json};

use super::{SyncEventsReqBody, SyncEventsResBody, request, response};

#[test]
fn v4_request_uses_body_parameters_and_the_new_list_contract() {
    let body = json!({
        "conn_id":"mobile", "pos":"opaque", "timeout":30000, "set_presence":"unavailable",
        "lists":{"main":{
            "range":[0,19], "required_state":{"include":[{"type":"m.room.name"},{}],
                "exclude":[{"type":"m.room.member","state_key":"@ignored:example"}],"lazy_members":true},
            "timeline_limit":20,
            "filters":{"is_dm":false,"is_encrypted":true,"is_invited":null,
                "room_types":[null,"m.space"],"not_room_types":["m.space"],
                "spaces":["!space:example"],"tags":["m.favourite"],"not_tags":["m.lowpriority"]}
        }},
        "room_subscriptions":{"!room:example":{"required_state":{},"timeline_limit":0}}
    });
    let parsed: SyncEventsReqBody = serde_json::from_value(body.clone()).unwrap();
    parsed.validate().unwrap();
    assert_eq!(parsed.timeout.unwrap().as_millis(), 30000);
    let filters = &parsed.lists["main"].filters;
    assert_eq!(filters.is_dm, request::FilterMode::Exclude);
    assert_eq!(filters.is_encrypted, request::FilterMode::Only);
    assert_eq!(filters.is_invited, request::FilterMode::Both);
    let mut expected = body;
    expected["lists"]["main"]["filters"]
        .as_object_mut()
        .unwrap()
        .remove("is_invited");
    assert_eq!(serde_json::to_value(parsed).unwrap(), expected);
    let all: request::List =
        serde_json::from_value(json!({"required_state":{},"timeline_limit":0})).unwrap();
    assert!(
        all.range.is_none(),
        "omitted range means all matching rooms"
    );
}

#[test]
fn v4_rejects_unsafe_integers_and_validates_ranges_and_cardinality() {
    for value in [json!(-1), json!(1.5), json!(9_007_199_254_740_992_u64)] {
        assert!(
            serde_json::from_value::<request::List>(
                json!({"timeline_limit":value,"required_state":{}})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<request::List>(
                json!({"timeline_limit":0,"required_state":{},"range":[0,value]})
            )
            .is_err()
        );
    }
    let mut body: SyncEventsReqBody = serde_json::from_value(json!({"lists":{"x":{
        "required_state":{},"timeline_limit":0,"range":[2,1]}}}))
    .unwrap();
    assert!(body.validate().is_err());
    body.lists.get_mut("x").unwrap().range = Some((2_u32.into(), 2_u32.into()));
    body.validate().unwrap();
    let template = body.lists["x"].clone();
    body.lists = (0..100)
        .map(|i| (i.to_string(), template.clone()))
        .collect();
    body.validate().unwrap();
    body.lists.insert("overflow".into(), template);
    assert!(body.validate().is_err());
    body.lists.clear();
    body.room_subscriptions = (0..100)
        .map(|i| {
            (
                crate::RoomId::parse(format!("!{i}:example")).unwrap(),
                request::RoomSubscription::new(request::RequiredState::new(), 0_u32.into()),
            )
        })
        .collect();
    body.validate().unwrap();
    body.room_subscriptions.insert(
        crate::RoomId::parse("!overflow:example").unwrap(),
        request::RoomSubscription::new(request::RequiredState::new(), 0_u32.into()),
    );
    assert!(body.validate().is_err());
}

#[test]
fn v4_distinguishes_omitted_cleared_full_and_stripped_room_data() {
    for body in [
        json!({}),
        json!({"name":null,"avatar":null,"heroes":[],"is_dm":false}),
        json!({"name":"name","avatar":"mxc://example/avatar","initial":true}),
        json!({"stripped_state":[]}),
        json!({"membership":"invite","stripped_state":[{"type":"m.room.name","state_key":"","sender":"@a:example","content":{"name":"Invite"}}]}),
    ] {
        let room: response::Room = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(
            matches!(room.data, response::RoomData::Stripped(_)),
            body.get("stripped_state").is_some()
        );
        assert_eq!(serde_json::to_value(room).unwrap(), body);
    }
    assert!(serde_json::from_value::<response::Room>(json!({"stripped_state":{}})).is_err());
}

#[test]
fn v4_deleted_state_stubs_are_not_empty_or_null_event_content() {
    for content in [None, Some(json!({})), Some(Value::Null)] {
        let mut event = json!({"type":"m.room.name","state_key":""});
        if let Some(content) = content {
            event["content"] = content;
        }
        let entry: response::RequiredStateEntry = serde_json::from_value(event.clone()).unwrap();
        assert_eq!(
            matches!(entry, response::RequiredStateEntry::Deleted(_)),
            event.get("content").is_none()
        );
        assert_eq!(serde_json::to_value(entry).unwrap(), event);
    }
    assert!(
        serde_json::from_value::<response::RequiredStateEntry>(json!({"type":"m.room.name"}))
            .is_err()
    );
}

#[test]
fn v4_preserves_unknown_extensions_and_safe_response_counters() {
    let request = json!({"extensions":{"com.example.future":{"enabled":true,"value":null}}});
    let parsed: SyncEventsReqBody = serde_json::from_value(request.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), request);
    let response = json!({"pos":"opaque","lists":{"main":{"count":9_007_199_254_740_991_u64}},
        "rooms":{"!r:example":{"num_live":0,"joined_count":0,"invited_count":0}},
        "extensions":{"com.example.future":{"value":null}}});
    let parsed: SyncEventsResBody = serde_json::from_value(response.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), response);
    assert!(
        serde_json::from_value::<SyncEventsResBody>(
            json!({"pos":"x","lists":{"main":{"count":9_007_199_254_740_992_u64}}})
        )
        .is_err()
    );
}
