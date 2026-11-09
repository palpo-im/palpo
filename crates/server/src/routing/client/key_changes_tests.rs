//! Real HTTP membership changes and immutable device-list token windows.
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedRoomId, OwnedUserId, UnixMillis, UserId};
use crate::{config, data};

async fn ok(request: RequestBuilder, service: &Service) -> Value {
    let router = service.router();
    let mut response = tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap();
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

fn get(url: String, token: &str) -> RequestBuilder {
    TestClient::get(url).add_header("Authorization", format!("Bearer {token}"), true)
}

async fn provision(localpart: &str) -> (OwnedUserId, String) {
    let user = UserId::parse(format!("@{localpart}:{}", config::server_name())).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: localpart.into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&user).await.unwrap();
    let token = format!("{localpart}-token");
    data::user::device::create_device(&user, &OwnedDeviceId::from("D"), &token, None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&user).await.unwrap();
    (user, token)
}

async fn create(service: &Service, token: &str, encrypted: bool) -> OwnedRoomId {
    let initial = if encrypted {
        json!([{"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}])
    } else {
        json!([])
    };
    let body = ok(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", format!("Bearer {token}"), true)
            .json(&json!({"room_version":"11","preset":"public_chat","initial_state":initial})),
        service,
    )
    .await;
    body["room_id"].as_str().unwrap().try_into().unwrap()
}

async fn action(service: &Service, room: &OwnedRoomId, token: &str, action: &str, body: Value) {
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/{action}"
        ))
        .add_header("Authorization", format!("Bearer {token}"), true)
        .json(&body),
        service,
    )
    .await;
}

async fn sync(service: &Service, token: &str, since: Option<&str>) -> Value {
    let query = since.map_or(String::new(), |since| format!("&since={since}"));
    ok(
        get(
            format!("http://localhost/_matrix/client/v3/sync?timeout=0{query}"),
            token,
        ),
        service,
    )
    .await
}

async fn changes(service: &Service, token: &str, from: &str, to: &str) -> Value {
    ok(
        get(
            format!("http://localhost/_matrix/client/v3/keys/changes?from={from}&to={to}"),
            token,
        ),
        service,
    )
    .await
}

fn listed(body: &Value, list: &str, user: &OwnedUserId) -> bool {
    body[list]
        .as_array()
        .is_some_and(|users| users.iter().any(|u| u.as_str() == Some(user.as_str())))
}

async fn assert_window(
    service: &Service,
    user: &OwnedUserId,
    token: &str,
    from: &str,
    peer: &OwnedUserId,
    left: bool,
) -> String {
    let response = sync(service, token, Some(from)).await;
    let to = response["next_batch"].as_str().unwrap().to_owned();
    let changes = changes(service, token, from, &to).await;
    assert_eq!(listed(&changes, "left", peer), left, "{changes}");
    assert_eq!(
        listed(&response["device_lists"], "left", peer),
        left,
        "{response}"
    );
    assert!(
        !listed(&changes, "left", user),
        "self must never leave device tracking"
    );
    // Sliding sync uses the same device-list window without requiring room
    // subscriptions. In particular, leaving one's own last room still reports left.
    let from: crate::event::BatchToken = from.parse().unwrap();
    let request = serde_json::from_value(json!({"extensions":{"e2ee":{"enabled":true}}})).unwrap();
    let sliding = crate::sync_v5::sync_events(
        user,
        "D".into(),
        from.event_sn(),
        &request,
        &Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        sliding.extensions.e2ee.device_lists.left.contains(peer),
        left
    );
    to
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn review_keys_changes_tracks_shared_encrypted_membership_windows() {
    std::thread::Builder::new()
        .name("key-changes-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn run() {
    crate::test_database::init();
    config::CONFIG.set(serde_json::from_value(json!({"server_name":"key-changes.example","db":{"url":"unused-test-config"},"ip_range_denylist":[],"auto_join_rooms":[]})).unwrap()).unwrap();
    let (alice, at) = provision("alice").await;
    let (bob, bt) = provision("bob").await;
    let (carol, ct) = provision("carol").await;
    let service = Service::new(crate::routing::root());
    let room = create(&service, &at, true).await;
    action(&service, &room, &bt, "join", json!({})).await;
    let initial = sync(&service, &at, None).await;
    let joined = initial["next_batch"].as_str().unwrap().to_owned();
    assert!(listed(&initial["device_lists"], "changed", &bob));
    action(&service, &room, &bt, "leave", json!({})).await;
    let left = assert_window(&service, &alice, &at, &joined, &bob, true).await;
    assert_window(&service, &bob, &bt, &joined, &alice, true).await;
    // A second device receives the same user-level leave independently.
    data::user::device::create_device(
        &alice,
        &OwnedDeviceId::from("D2"),
        "alice-second",
        None,
        None,
    )
    .await
    .unwrap();
    assert!(listed(
        &changes(&service, "alice-second", &joined, &left).await,
        "left",
        &bob
    ));
    action(&service, &room, &bt, "join", json!({})).await;
    let rejoined = assert_window(&service, &alice, &at, &left, &bob, false).await;
    assert!(listed(
        &changes(&service, &at, &left, &rejoined).await,
        "changed",
        &bob
    ));
    // Historical endpoints remain frozen after rejoining; a leave+rejoin in
    // one window ends tracking no user, whereas adjacent windows report both.
    assert!(listed(
        &changes(&service, &at, &joined, &left).await,
        "left",
        &bob
    ));
    assert!(!listed(
        &changes(&service, &at, &joined, &rejoined).await,
        "left",
        &bob
    ));
    let second = create(&service, &at, true).await;
    action(&service, &second, &bt, "join", json!({})).await;
    let both = sync(&service, &at, None).await["next_batch"]
        .as_str()
        .unwrap()
        .to_owned();
    action(&service, &room, &bt, "leave", json!({})).await;
    let one = assert_window(&service, &alice, &at, &both, &bob, false).await;
    let plain_bob = create(&service, &at, false).await;
    action(&service, &plain_bob, &bt, "join", json!({})).await;
    action(&service, &second, &at, "ban", json!({"user_id":bob})).await;
    assert_window(&service, &alice, &at, &one, &bob, true).await;
    // Plaintext sharing does not keep encrypted tracking alive, but device
    // updates for a currently joined plaintext peer must still be reported.
    let plain = create(&service, &at, false).await;
    let plain_before = sync(&service, &at, None).await["next_batch"]
        .as_str()
        .unwrap()
        .to_owned();
    action(&service, &plain, &ct, "join", json!({})).await;
    let plain_joined = sync(&service, &at, Some(&plain_before)).await;
    assert!(listed(&plain_joined["device_lists"], "changed", &carol));
    let p0 = plain_joined["next_batch"].as_str().unwrap().to_owned();
    crate::user::mark_device_key_update(&carol, "C1".into())
        .await
        .unwrap();
    let p1 = sync(&service, &at, Some(&p0)).await;
    let p1_token = p1["next_batch"].as_str().unwrap().to_owned();
    assert!(listed(&p1["device_lists"], "changed", &carol));
    crate::user::mark_device_key_update(&carol, "C2".into())
        .await
        .unwrap();
    let p2 = sync(&service, &at, Some(&p1_token)).await;
    let p2_token = p2["next_batch"].as_str().unwrap();
    assert!(
        listed(
            &changes(&service, &at, &p0, &p1_token).await,
            "changed",
            &carol
        ),
        "later key updates must not erase a historical window"
    );
    assert!(listed(
        &changes(&service, &at, &p1_token, p2_token).await,
        "changed",
        &carol
    ));
    let empty = changes(&service, &at, &p1_token, &p1_token).await;
    assert_eq!(empty, json!({"changed":[],"left":[]}));
    let suffix_from = format!("{p0}_i1_pabc_wxyz");
    assert!(listed(
        &changes(&service, &at, &suffix_from, &p1_token).await,
        "changed",
        &carol
    ));
    // Equality excludes a marker at the upper next-position, and includes it
    // in the following window. Explicit sync tokens may retain legacy s form.
    let before = data::curr_sn().await.unwrap() + 1;
    crate::user::mark_device_key_update(&alice, "D".into())
        .await
        .unwrap();
    let after = data::curr_sn().await.unwrap() + 1;
    assert!(!listed(
        &changes(&service, &at, &format!("s{before}"), &format!("p{before}")).await,
        "changed",
        &alice
    ));
    assert!(listed(
        &changes(&service, &at, &format!("p{before}"), &format!("p{after}")).await,
        "changed",
        &alice
    ));
    // A cached signed outlier has an old event identity, but only starts
    // sharing when its live-publication frame is committed later.
    let delayed = create(&service, &at, true).await;
    let before_frame = crate::room::get_frame_id(&delayed, None).await.unwrap();
    let pre_cache = sync(&service, &at, None).await["next_batch"]
        .as_str()
        .unwrap()
        .to_owned();
    let (join_pdu, mut join_json) = crate::event::PduBuilder {
        event_type: "m.room.member".into(),
        state_key: Some(bob.to_string()),
        content: serde_json::value::to_raw_value(&json!({"membership":"join"})).unwrap(),
        ..Default::default()
    }
    .hash_sign(&bob, &delayed, &crate::core::RoomVersionId::V11)
    .await
    .unwrap();
    let (cached, _, guard) =
        crate::event::PduBuilder::save_as_outlier(join_pdu, join_json.clone(), &bob)
            .await
            .unwrap();
    crate::event::update_frame_id(&cached.event_id, before_frame)
        .await
        .unwrap();
    drop(guard);
    let old = sync(&service, &at, None).await["next_batch"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(cached.event_sn < old.parse::<crate::event::BatchToken>().unwrap().event_sn());
    join_json.remove("event_id");
    crate::federation::membership::send_join_v2(
        config::server_name(),
        &delayed,
        &serde_json::value::to_raw_value(&join_json).unwrap(),
    )
    .await
    .unwrap();
    let published = assert_window(&service, &alice, &at, &old, &bob, false).await;
    assert!(listed(
        &changes(&service, &at, &old, &published).await,
        "changed",
        &bob
    ));
    assert!(
        !listed(
            &changes(&service, &at, &pre_cache, &old).await,
            "changed",
            &bob
        ),
        "unpublished identity must not grant encrypted sharing"
    );
    action(&service, &delayed, &at, "leave", json!({})).await;
    assert_window(&service, &alice, &at, &published, &bob, true).await;
    // A later own departure must not alter the earlier joined endpoint.
    assert!(listed(
        &changes(&service, &at, &old, &published).await,
        "changed",
        &bob
    ));
    // Reversed intervals and messages-page cursors cannot masquerade as sync positions.
    for (from, to) in [
        (p2_token.to_owned(), p0.clone()),
        ("q0.1.1.1".into(), p1_token.clone()),
        ("t1-1".into(), p1_token),
    ] {
        let mut response = get(
            format!("http://localhost/_matrix/client/v3/keys/changes?from={from}&to={to}"),
            &at,
        )
        .send(&service)
        .await;
        let status = response.status_code;
        let body = response.take_json::<Value>().await.unwrap();
        assert_eq!(status, Some(StatusCode::BAD_REQUEST), "{body}");
    }
}
