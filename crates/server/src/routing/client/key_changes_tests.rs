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

async fn put_state(
    service: &Service,
    room: &OwnedRoomId,
    token: &str,
    ty: &str,
    key: &str,
    content: Value,
) {
    ok(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/{ty}/{key}"
        ))
        .add_header("Authorization", format!("Bearer {token}"), true)
        .json(&content),
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

fn sync_lists(body: &Value) -> crate::core::device::DeviceLists {
    serde_json::from_value(
        body.get("device_lists")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .expect("sync device lists must satisfy their response schema")
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

async fn out_of_order_key_history(
    service: &Service,
    token: &str,
    peer: &OwnedUserId,
    room: &OwnedRoomId,
) {
    use diesel::prelude::*;
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};

    use crate::data::schema::e2e_key_changes;

    let older = data::next_sn().await.unwrap();
    let newer = data::next_sn().await.unwrap();
    assert!(older < newer);
    for (occur_sn, changed_at) in [(newer, 200), (older, 100), (older, 100)] {
        data::user::key::replace_key_change(&data::user::NewDbKeyChange {
            user_id: peer.clone(),
            room_id: Some(room.clone()),
            occur_sn,
            changed_at: UnixMillis(changed_at),
        })
        .await
        .unwrap();
    }
    let mut conn = data::connect().await.unwrap();
    let latest = e2e_key_changes::table
        .filter(e2e_key_changes::user_id.eq(peer))
        .filter(e2e_key_changes::room_id.eq(room))
        .select((e2e_key_changes::occur_sn, e2e_key_changes::changed_at))
        .first::<(i64, UnixMillis)>(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        latest,
        (newer, UnixMillis(200)),
        "delayed update moved latest marker backwards"
    );

    #[derive(QueryableByName)]
    struct JournalCount {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let count = diesel::sql_query(
        "SELECT count(*) FROM e2e_key_change_history WHERE user_id = $1 AND occur_sn IN ($2, $3)",
    )
    .bind::<diesel::sql_types::Text, _>(peer.as_str())
    .bind::<diesel::sql_types::BigInt, _>(older)
    .bind::<diesel::sql_types::BigInt, _>(newer)
    .get_result::<JournalCount>(&mut conn)
    .await
    .unwrap();
    assert_eq!(
        count.count, 2,
        "upsert trigger or replay duplicated a journal position"
    );
    drop(conn);
    for position in [older, newer] {
        assert!(
            listed(
                &changes(
                    service,
                    token,
                    &format!("p{position}"),
                    &format!("p{}", position + 1)
                )
                .await,
                "changed",
                peer,
            ),
            "out-of-order committed key change disappeared from its historical interval"
        );
    }

    let rolled_back = data::next_sn().await.unwrap();
    let mut conn = data::connect().await.unwrap();
    conn.batch_execute("BEGIN").await.unwrap();
    diesel::insert_into(e2e_key_changes::table)
        .values(data::user::NewDbKeyChange {
            user_id: peer.clone(),
            room_id: Some(room.clone()),
            occur_sn: rolled_back,
            changed_at: UnixMillis(300),
        })
        .on_conflict((e2e_key_changes::user_id, e2e_key_changes::room_id))
        .do_update()
        .set((
            e2e_key_changes::occur_sn.eq(rolled_back),
            e2e_key_changes::changed_at.eq(UnixMillis(300)),
        ))
        .execute(&mut conn)
        .await
        .unwrap();
    let count = diesel::sql_query(
        "SELECT count(*) FROM e2e_key_change_history WHERE user_id = $1 AND occur_sn = $2",
    )
    .bind::<diesel::sql_types::Text, _>(peer.as_str())
    .bind::<diesel::sql_types::BigInt, _>(rolled_back)
    .get_result::<JournalCount>(&mut conn)
    .await
    .unwrap();
    assert_eq!(
        count.count, 1,
        "INSERT and UPDATE trigger paths must deduplicate in the transaction"
    );
    conn.batch_execute("ROLLBACK").await.unwrap();
    let latest = e2e_key_changes::table
        .filter(e2e_key_changes::user_id.eq(peer))
        .filter(e2e_key_changes::room_id.eq(room))
        .select(e2e_key_changes::occur_sn)
        .first::<i64>(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        latest, newer,
        "rollback retained an uncommitted latest marker"
    );
    drop(conn);
    assert!(
        !listed(
            &changes(
                service,
                token,
                &format!("p{rolled_back}"),
                &format!("p{}", rolled_back + 1)
            )
            .await,
            "changed",
            peer,
        ),
        "rollback retained an uncommitted journal record"
    );
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
    out_of_order_key_history(&service, &at, &carol, &plain).await;
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
    // Enabling encryption starts tracking for both participants, including
    // their own devices, even though they already shared this plaintext room.
    let alice_plain = sync(&service, &at, None).await;
    let carol_plain = sync(&service, &ct, None).await;
    put_state(
        &service,
        &plain,
        &at,
        "m.room.encryption",
        "",
        json!({"algorithm":"m.megolm.v1.aes-sha2"}),
    )
    .await;
    let alice_encrypted = sync(&service, &at, alice_plain["next_batch"].as_str()).await;
    let carol_encrypted = sync(&service, &ct, carol_plain["next_batch"].as_str()).await;
    assert_eq!(
        alice_encrypted["device_lists"]["changed"],
        json!([alice, carol])
    );
    assert_eq!(
        carol_encrypted["device_lists"]["changed"],
        json!([alice, carol])
    );
    put_state(
        &service,
        &plain,
        &ct,
        "m.room.member",
        carol.as_str(),
        json!({"membership":"join","displayname":"profile-only update"}),
    )
    .await;
    let profile = sync(&service, &ct, carol_encrypted["next_batch"].as_str()).await;
    assert!(
        sync_lists(&profile).is_empty(),
        "profile updates must retain the same joined period"
    );
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
    assert_eq!(
        changes(&service, &at, &old, &published).await["changed"],
        json!([bob])
    );
    let imported_join = sync(&service, &bt, Some(&old)).await;
    let imported_until: crate::event::BatchToken = imported_join["next_batch"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        data::user::key::keys_changed_in_window(
            std::slice::from_ref(&bob),
            old.parse::<crate::event::BatchToken>().unwrap().event_sn(),
            imported_until.event_sn(),
        )
        .await
        .unwrap()
        .is_empty(),
        "imported join fixture must not rely on an own-device marker"
    );
    assert_eq!(
        imported_join["device_lists"]["changed"],
        json!([alice, bob]),
        "joining user's sync must include peers and self even without a key upload"
    );
    assert!(sync_lists(&imported_join).left.is_empty());
    let acknowledged = sync(&service, &bt, imported_join["next_batch"].as_str()).await;
    assert!(
        sync_lists(&acknowledged).is_empty(),
        "own join notification must not repeat after acknowledgement"
    );
    // Rejoining inside one sync window has the same room at both endpoints
    // but a new joined period. Use the federation importer again, without the
    // local /join device-marker side effect.
    let rejoin_from = acknowledged["next_batch"].as_str().unwrap();
    action(&service, &delayed, &bt, "leave", json!({})).await;
    let (_, mut joining) = crate::event::PduBuilder {
        event_type: "m.room.member".into(),
        state_key: Some(bob.to_string()),
        content: serde_json::value::to_raw_value(&json!({"membership":"join"})).unwrap(),
        ..Default::default()
    }
    .hash_sign(&bob, &delayed, &crate::core::RoomVersionId::V11)
    .await
    .unwrap();
    joining.remove("event_id");
    crate::federation::membership::send_join_v2(
        config::server_name(),
        &delayed,
        &serde_json::value::to_raw_value(&joining).unwrap(),
    )
    .await
    .unwrap();
    let own_rejoin = sync(&service, &bt, Some(rejoin_from)).await;
    let own_lists = sync_lists(&own_rejoin);
    assert!(
        own_lists.changed.contains(&bob),
        "same-window rejoin must refresh the caller's devices"
    );
    assert!(!own_lists.left.contains(&bob));
    let rejoin_until: crate::event::BatchToken =
        own_rejoin["next_batch"].as_str().unwrap().parse().unwrap();
    assert!(
        data::user::key::keys_changed_in_window(
            std::slice::from_ref(&bob),
            rejoin_from
                .parse::<crate::event::BatchToken>()
                .unwrap()
                .event_sn(),
            rejoin_until.event_sn()
        )
        .await
        .unwrap()
        .is_empty(),
        "same-window imported rejoin must not rely on an own-device marker"
    );
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
    let resource_room = create(&service, &at, true).await;
    action(&service, &resource_room, &bt, "join", json!({})).await;
    crate::user::device_lists::assert_large_custom_state_is_not_resolved(
        &resource_room,
        &alice,
        &bob,
    )
    .await;
}
