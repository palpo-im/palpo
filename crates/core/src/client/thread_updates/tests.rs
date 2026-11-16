use serde_json::json;

use super::*;

#[test]
fn query_tokens_are_opaque_and_only_positive_backward_pagination_is_valid() {
    let args: ThreadUpdatesReqArgs =
        serde_html_form::from_str("dir=b&from=a%2Fb%2B%3D&to=end%26boundary&limit=10").unwrap();
    args.validate().unwrap();
    assert_eq!(args.from.as_deref(), Some("a/b+="));
    assert_eq!(args.to.as_deref(), Some("end&boundary"));
    assert_eq!(args.limit, Some(10_u32.into()));
    assert_eq!(
        serde_json::to_value(ThreadUpdatesReqArgs::new()).unwrap(),
        json!({"dir":"b"})
    );
    for query in ["dir=f", "dir=b&limit=0"] {
        let args: ThreadUpdatesReqArgs = serde_html_form::from_str(query).unwrap();
        assert!(args.validate().is_err());
    }
    for query in [
        "limit=1",
        "dir=b&limit=-1",
        "dir=b&limit=1.5",
        "dir=b&limit=9007199254740992",
    ] {
        assert!(
            serde_html_form::from_str::<ThreadUpdatesReqArgs>(query).is_err(),
            "{query}"
        );
    }
}

#[test]
fn sync_extension_enablement_and_pagination_only_updates_survive_serialization() {
    use crate::client::sync_events::v4::{SyncEventsReqBody, SyncEventsResBody};
    assert_eq!(
        serde_json::to_value(SyncEventsReqBody::default()).unwrap(),
        json!({})
    );
    for include_roots in [false, true] {
        let wire = json!({"extensions":{(EXTENSION):{"enabled":true,"include_roots":include_roots,"limit":10}}});
        let body: SyncEventsReqBody = serde_json::from_value(wire.clone()).unwrap();
        assert!(!body.extensions.is_empty());
        assert_eq!(body.extensions.threads.include_roots, include_roots);
        assert_eq!(serde_json::to_value(body).unwrap(), wire);
    }
    let disabled: SyncEventsReqBody = serde_json::from_value(json!({"extensions":{(EXTENSION):{
        "enabled":false,"include_roots":true,"limit":10}}}))
    .unwrap();
    assert_eq!(serde_json::to_value(disabled).unwrap(), json!({}));
    let response =
        json!({"pos":"sync-token","extensions":{(EXTENSION):{"prev_batch":"cross-room"}}});
    let parsed: SyncEventsResBody = serde_json::from_value(response.clone()).unwrap();
    assert!(
        !parsed.extensions.is_empty(),
        "a cursor without roots must not disappear"
    );
    assert_eq!(serde_json::to_value(parsed).unwrap(), response);
}

#[test]
fn sync_and_pagination_keep_distinct_cross_room_and_relations_tokens() {
    let root = json!({"event_id":"$a","type":"m.room.message","sender":"@a:example","content":{"body":"root","msgtype":"m.text"},
        "unsigned":{"m.relations":{"m.thread":{"count":2,"latest_event":{"event_id":"$reply"}}}}});
    let sync = json!({"updates":{
        "!a:example":{"$a":{"thread_root":root,"prev_batch":"relations-a"}},
        "!b:example":{"$b":{"prev_batch":"relations-b"},"$c":{}}
    },"prev_batch":"cross-room"});
    let parsed: Threads = serde_json::from_value(sync.clone()).unwrap();
    assert_eq!(parsed.updates.len(), 2);
    let room = crate::RoomId::parse("!b:example").unwrap();
    let event = crate::EventId::parse("$c").unwrap();
    assert!(parsed.updates[&room][&event].thread_root.is_none());
    assert_eq!(serde_json::to_value(parsed).unwrap(), sync);
    let page = json!({"chunk":{"!a:example":{"$a":{"thread_root":root,"prev_batch":"relations-a"}}},"next_batch":"next-page"});
    let parsed: ThreadUpdatesResBody = serde_json::from_value(page.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), page);
    assert!(
        serde_json::from_value::<ThreadUpdatesResBody>(json!({"chunk":{"!a:example":{"$a":{}}}}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<Threads>(json!({"updates":{"invalid room":{"$a":{}}}})).is_err()
    );
    assert_eq!(
        serde_json::to_value(ThreadUpdatesResBody::default()).unwrap(),
        json!({})
    );
}
