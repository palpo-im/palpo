#![cfg(feature = "unstable-msc4480")]
use palpo_core::client::sync_events::v5::{ExtensionsConfig, SyncEventsResBody};
use serde_json::json;

#[test]
fn sticky_extension_has_selectors_without_an_independent_cursor() {
    let wire = json!({"org.matrix.msc4354.sticky_events":{
        "enabled":true,"limit":1,"lists":[],"rooms":["*","!a:example.org"]
    }});
    let extensions: ExtensionsConfig = serde_json::from_value(wire.clone()).unwrap();
    assert!(!extensions.is_empty());
    assert_eq!(serde_json::to_value(extensions).unwrap(), wire);
    assert!(
        serde_json::from_value::<ExtensionsConfig>(
            json!({"org.matrix.msc4354.sticky_events":{"enabled":true,"limit":-1}})
        )
        .is_err()
    );
    let mut response = SyncEventsResBody::new("42".into());
    response.extensions = serde_json::from_value(json!({
        "org.matrix.msc4354.sticky_events":{"rooms":{"!a:example.org":{"events":[{
            "event_id":"$sticky", "type":"m.room.message","sender":"@a:example.org",
            "origin_server_ts":1,"content":{"msgtype":"m.text","body":"sticky"}
        }]}}}
    }))
    .unwrap();
    assert!(!response.extensions.is_empty());
    assert!(!response.extensions.is_empty_for_long_poll());
    let wire = serde_json::to_value(response).unwrap();
    let sticky = &wire["extensions"]["org.matrix.msc4354.sticky_events"];
    assert!(sticky.get("since").is_none() && sticky.get("next_batch").is_none());
}
