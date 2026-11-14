#[cfg(feature = "unstable-msc3401")]
use palpo_core::events::call::member::{CallMemberEventContent, Focus};
#[cfg(feature = "unstable-msc3401")]
use serde_json::Value;
#[cfg(any(feature = "unstable-msc3401", feature = "unstable-msc4143"))]
use serde_json::json;

#[cfg(feature = "unstable-msc3401")]
fn session(foci: Option<Value>) -> Value {
    let mut wire = json!({"application":"m.call", "call_id":"call", "scope":"m.room",
        "device_id":"DEVICE", "expires":14400000,
        "focus_active":{"type":"livekit","focus_selection":"oldest_membership"}});
    if let Some(foci) = foci {
        wire["foci_preferred"] = foci;
    }
    wire
}

#[cfg(feature = "unstable-msc3401")]
#[test]
fn rtc_sessions_survive_current_legacy_unknown_and_missing_foci() {
    for foci in [
        None,
        Some(json!([])),
        Some(json!([
            {"type":"m.livekit", "url":"wss://sfu.example", "server_name":"other.example"},
            {"type":"livekit", "livekit_service_url":"https://jwt.example", "livekit_alias":"room"},
            {"type":"org.example.future", "nested":{"values":[1,true,"hello"]}},
        ])),
    ] {
        let wire = session(foci.clone());
        let parsed: CallMemberEventContent = serde_json::from_value(wire.clone()).unwrap();
        let CallMemberEventContent::SessionContent(parsed) = parsed else {
            panic!("a session must not be mistaken for an empty leave event: {wire}");
        };
        assert_eq!(parsed.device_id.as_str(), "DEVICE");
        let encoded = serde_json::to_value(&parsed).unwrap();
        assert_eq!(encoded["foci_preferred"], foci.unwrap_or_else(|| json!([])));
        for focus in parsed.foci_preferred {
            let raw = serde_json::to_value(&focus).unwrap();
            assert_eq!(focus.focus_type(), raw["type"].as_str().unwrap());
            assert!(!focus.data().contains_key("type"));
        }
    }
}

#[cfg(feature = "unstable-msc3401")]
#[test]
fn rtc_focus_parsing_keeps_legacy_fields_but_rejects_invalid_known_data() {
    // Current upstream permits service-only focus data for both names.
    for ty in ["livekit", "m.livekit"] {
        let raw = json!({"type":ty,"livekit_service_url":"https://jwt.example"});
        let focus: Focus = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(focus).unwrap(), raw);
        assert!(
            serde_json::from_value::<Focus>(json!({
                "type":ty,"server_name":"~"
            }))
            .is_err()
        );
    }
    for raw in [json!({}), json!({"type":2}), json!({"type":null})] {
        assert!(serde_json::from_value::<Focus>(raw).is_err());
    }
}

#[cfg(feature = "unstable-msc4195")]
#[test]
#[allow(deprecated)]
fn rtc_transports_distinguish_stable_legacy_and_unknown_contracts() {
    use palpo_core::client::rtc::transports::{LiveKitRtcTransport, RtcTransport};
    for wire in [
        json!({"type":"m.livekit","url":"wss://sfu.example"}),
        json!({"type":"livekit","livekit_service_url":"https://jwt.example"}),
        json!({"type":"livekit","url":"wss://sfu.example","livekit_service_url":"https://jwt.example"}),
        json!({"type":"custom","data":{"nested":[1,2]}}),
    ] {
        let parsed: RtcTransport = serde_json::from_value(wire.clone()).unwrap();
        match wire["type"].as_str().unwrap() {
            "m.livekit" => assert!(matches!(&parsed, RtcTransport::LiveKit(_))),
            "livekit" => assert!(matches!(&parsed, RtcTransport::LegacyLiveKit(_))),
            _ => assert!(matches!(&parsed, RtcTransport::_Custom(_))),
        }
        assert_eq!(parsed.transport_type(), wire["type"].as_str().unwrap());
        assert!(!parsed.data().contains_key("type"));
        assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
    }
    let wire = json!({"type":"m.livekit","livekit_service_url":"https://jwt.example"});
    assert!(serde_json::from_value::<RtcTransport>(wire).is_err());
    assert!(RtcTransport::new("m.livekit", serde_json::Map::new()).is_err());
    for raw in [json!({}), json!({"type":false})] {
        assert!(serde_json::from_value::<RtcTransport>(raw).is_err());
    }
    let current = RtcTransport::from(LiveKitRtcTransport::new("wss://sfu.example".into()));
    assert_eq!(current.transport_type(), "m.livekit");
    let legacy = RtcTransport::from(LiveKitRtcTransport::with_service_url(
        "https://jwt.example".into(),
    ));
    assert_eq!(legacy.transport_type(), "livekit");
    let alias: RtcTransport = serde_json::from_value(json!({
        "type":"livekit_multi_sfu","livekit_service_url":"https://jwt.example"
    }))
    .unwrap();
    assert_eq!(alias.transport_type(), "livekit");
}

#[cfg(feature = "unstable-msc4195")]
#[test]
#[allow(deprecated)]
fn rtc_discovery_conversion_never_advertises_service_only_as_current_transport() {
    use palpo_core::client::discovery::homeserver::LiveKitRtcFocusInfo;
    use palpo_core::client::rtc::transports::RtcTransport;
    for (raw, expected) in [
        (
            json!({"livekit_service_url":"https://jwt.example"}),
            "livekit",
        ),
        (
            json!({"url":"wss://sfu.example","livekit_service_url":"https://jwt.example"}),
            "m.livekit",
        ),
    ] {
        let info: LiveKitRtcFocusInfo = serde_json::from_value(raw).unwrap();
        let transport = RtcTransport::from(info);
        assert_eq!(transport.transport_type(), expected);
    }
}

#[cfg(feature = "unstable-msc4143")]
#[test]
fn custom_transport_roundtrips_with_or_without_livekit_feature() {
    use palpo_core::client::rtc::transports::RtcTransport;
    let wire = json!({"type":"org.example.future","nested":{"items":[1,true]}});
    let transport: RtcTransport = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(transport.transport_type(), "org.example.future");
    assert_eq!(serde_json::to_value(transport).unwrap(), wire);
    for wire in [json!({}), json!({"type":1}), json!([])] {
        assert!(serde_json::from_value::<RtcTransport>(wire).is_err());
    }
}
