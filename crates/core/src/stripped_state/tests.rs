use serde_json::{from_value, json, to_value};

use super::*;
use crate::client::state::stripped_state::{
    StrippedStateMsc4560ReqArgs as ClientArgs, StrippedStateMsc4560ResBody as ClientResponse,
};
use crate::error::ErrorKind;
use crate::federation::event::stripped_state::{
    StrippedStateMsc4560ReqArgs as FederationArgs,
    StrippedStateMsc4560ResBody as FederationResponse,
};
use crate::metadata::FeatureFlag;

#[test]
fn empty_and_count_only_requests_share_the_same_wire_format() {
    let empty: StrippedStateMsc4560ReqBody = from_value(json!({})).unwrap();
    assert!(empty.is_empty());
    empty.validate().unwrap();
    assert_eq!(to_value(empty).unwrap(), json!({}));

    let count: StrippedStateMsc4560ReqBody =
        from_value(json!({"num_joined_members": true})).unwrap();
    assert!(!count.is_empty());
    count.validate().unwrap();
    assert_eq!(
        to_value(count).unwrap(),
        json!({"num_joined_members": true})
    );
}

#[test]
fn exact_proposal_allowlist_accepts_space_relations_and_deduplicates_types() {
    let types = json!([
        "m.room.create",
        "m.room.name",
        "m.room.avatar",
        "m.room.topic",
        "m.room.join_rules",
        "m.room.canonical_alias",
        "m.room.guest_access",
        "m.room.history_visibility",
        "m.room.encryption",
        "m.space.child",
        "m.space.parent"
    ]);
    let body: StrippedStateMsc4560ReqBody = from_value(json!({"state": types})).unwrap();
    assert_eq!(body.state.len(), 11);
    assert_eq!(ALLOWED_STRIPPED_STATE_EVENTS.len(), 11);
    body.validate().unwrap();
    let repeated: StrippedStateMsc4560ReqBody =
        from_value(json!({"state": ["m.room.name", "m.room.name"]})).unwrap();
    assert_eq!(repeated.state.len(), 1);
    assert!(!repeated.is_empty());
}

#[test]
fn forbidden_and_unknown_types_produce_invalid_param_after_parsing() {
    for kind in [
        "m.room.member",
        "m.room.power_levels",
        "m.room.server_acl",
        "org.example.secret",
    ] {
        let body: StrippedStateMsc4560ReqBody = from_value(json!({"state": [kind]})).unwrap();
        assert!(matches!(
            body.validate().unwrap_err().kind,
            ErrorKind::InvalidParam
        ));
    }
}

#[test]
fn via_limit_counts_duplicates_before_deduplication() {
    for room in ["!room:example.org", "#room:example.org"] {
        let mut args: ClientArgs = from_value(json!({
            "room_id_or_alias": room,
            "via": vec!["example.org"; MAX_VIAS]
        }))
        .unwrap();
        args.validate().unwrap();
        args.via.push("example.org".try_into().unwrap());
        let error = args.validate().unwrap_err();
        assert!(matches!(error.kind, ErrorKind::TooLarge));
        assert_eq!(
            error.status_code,
            Some(salvo::http::StatusCode::BAD_REQUEST)
        );
    }
    let empty: ClientArgs = from_value(json!({"room_id_or_alias": "!room:example.org"})).unwrap();
    assert!(empty.via.is_empty());
    empty.validate().unwrap();
}

#[test]
fn identifiers_and_member_count_types_are_checked() {
    assert!(from_value::<ClientArgs>(json!({"room_id_or_alias": "not-a-room"})).is_err());
    assert!(
        from_value::<ClientArgs>(
            json!({"room_id_or_alias": "!room:example.org", "via": ["bad/server"]})
        )
        .is_err()
    );
    assert!(from_value::<FederationArgs>(json!({"room_id": "#alias:example.org"})).is_err());
    assert!(from_value::<FederationArgs>(json!({"room_id": "!room:example.org"})).is_ok());
    for count in [json!(-1), json!("1"), json!(1.5)] {
        assert!(
            from_value::<FederationResponse>(json!({"state": [], "num_joined_members": count}))
                .is_err()
        );
    }
}

#[test]
fn responses_omit_unrequested_counts_but_keep_zero_and_nonempty_state_keys() {
    let client = ClientResponse::new("example.org".try_into().unwrap());
    assert_eq!(
        to_value(client).unwrap(),
        json!({"state": [], "via": "example.org"})
    );
    assert!(from_value::<ClientResponse>(json!({"state": []})).is_err());
    assert_eq!(
        to_value(FederationResponse::default()).unwrap(),
        json!({"state": []})
    );

    let event = json!({
        "type": "m.space.child", "state_key": "!child:example.org",
        "sender": "@alice:example.org", "content": {"via": ["example.org"]}
    });
    let wire = json!({"state": [event], "num_joined_members": 0});
    let federation: FederationResponse = from_value(wire.clone()).unwrap();
    // Typed decoding confirms this is a stripped space relation, not an empty-key
    // room event. A handler must still sanitize untrusted raw events separately.
    federation.state[0].deserialize().unwrap();
    assert_eq!(to_value(federation).unwrap(), wire);
}

#[test]
fn capability_has_the_proposal_name() {
    assert_eq!(FeatureFlag::Msc4560.as_str(), "org.continuwuity.msc4560");
}
