use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedRoomId, OwnedUserId, UnixMillis, UserId};
use crate::event::BatchToken;
use crate::{config, data, room};

async fn send(request: RequestBuilder, service: &Service) -> Response {
    let router = service.router();
    tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap()
}

async fn ok(request: RequestBuilder, service: &Service) -> Value {
    let mut response = send(request, service).await;
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
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
    let token = format!("{localpart}-token");
    data::user::device::create_device(&user, &OwnedDeviceId::from("D"), &token, None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&user).await.unwrap();
    (user, token)
}

async fn sync(service: &Service, token: &str, since: Option<&str>) -> Value {
    let mut url = url::Url::parse("http://localhost/_matrix/client/v3/sync").unwrap();
    url.query_pairs_mut()
        .append_pair("timeout", "0")
        .append_pair(
            "filter",
            &json!({"room":{"timeline":{"limit":1}}}).to_string(),
        );
    if let Some(since) = since {
        url.query_pairs_mut().append_pair("since", since);
    }
    ok(
        TestClient::get(url.as_str()).add_header("Authorization", format!("Bearer {token}"), true),
        service,
    )
    .await
}

fn members_request(room: &OwnedRoomId, token: &str, at: &str) -> RequestBuilder {
    let mut url = url::Url::parse(&format!(
        "http://localhost/_matrix/client/v3/rooms/{room}/members"
    ))
    .unwrap();
    url.query_pairs_mut().append_pair("at", at);
    TestClient::get(url.as_str()).add_header("Authorization", format!("Bearer {token}"), true)
}

fn member<'a>(body: &'a Value, user: &OwnedUserId) -> Option<&'a Value> {
    body["chunk"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["state_key"] == user.as_str())
}

fn snapshot(body: &Value) -> std::collections::BTreeMap<String, (Value, Value)> {
    body["chunk"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            (
                event["state_key"].as_str().unwrap().to_owned(),
                (event["event_id"].clone(), event["content"].clone()),
            )
        })
        .collect()
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_members_publication_points_preserve_snapshot_and_leave_cap() {
    std::thread::Builder::new()
        .name("members-publication-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(members_publication());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn members_publication() {
    crate::test_database::init();
    // Other database_ tests may already have initialized this immutable config.
    // All fixture identities use its actual server and a unique localpart.
    let _ = config::CONFIG.set(
        serde_json::from_value(json!({
            "server_name":"members-publication.example", "db":{"url":"unused-test-config"},
            "ip_range_denylist":[], "auto_join_rooms":[]
        }))
        .unwrap(),
    );
    let prefix = format!("members-p-{}", uuid::Uuid::new_v4().simple());
    let (alice, alice_token) = provision(&format!("{prefix}-alice")).await;
    let (bob, bob_token) = provision(&format!("{prefix}-bob")).await;
    let (charlie, charlie_token) = provision(&format!("{prefix}-charlie")).await;
    let (_, keeper_token) = provision(&format!("{prefix}-keeper")).await;
    let service = Service::new(crate::routing::root());
    let created = ok(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", format!("Bearer {alice_token}"), true)
            .json(&json!({"room_version":"11", "preset":"public_chat"})),
        &service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    // Keep local participation while Alice and Bob both leave below.
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/join"
        ))
        .add_header("Authorization", format!("Bearer {keeper_token}"), true)
        .json(&json!({})),
        &service,
    )
    .await;
    let before = sync(&service, &alice_token, None).await;
    let previous = before["next_batch"].as_str().unwrap();
    assert!(matches!(
        previous.parse().unwrap(),
        BatchToken::Publication { .. }
    ));
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/join"
        ))
        .add_header("Authorization", format!("Bearer {bob_token}"), true)
        .json(&json!({})),
        &service,
    )
    .await;
    let bob_join = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        bob.as_str(),
        None,
    )
    .await
    .unwrap();
    let joined = sync(&service, &alice_token, Some(previous)).await;
    let timeline = &joined["rooms"]["join"][room.as_str()]["timeline"];
    assert_eq!(timeline["events"].as_array().unwrap().len(), 1, "{joined}");
    assert_eq!(timeline["events"][0]["state_key"], bob.as_str(), "{joined}");
    let prev = timeline["prev_batch"].as_str().unwrap();
    let next = joined["next_batch"].as_str().unwrap();
    assert!(matches!(
        prev.parse().unwrap(),
        BatchToken::Publication { .. }
    ));
    assert!(matches!(
        next.parse().unwrap(),
        BatchToken::Publication { .. }
    ));
    let before_join = ok(members_request(&room, &alice_token, prev), &service).await;
    assert!(member(&before_join, &bob).is_none(), "{before_join}");
    assert_eq!(
        member(&before_join, &alice).unwrap()["content"]["membership"],
        "join"
    );
    let after_join = ok(members_request(&room, &alice_token, next), &service).await;
    assert_eq!(
        member(&after_join, &bob).unwrap()["content"]["membership"],
        "join"
    );

    // Both Alice and Charlie join after Bob's departure. Bob's future token
    // must still resolve Alice's old leave and must not reveal Charlie at all.
    for token in [&alice_token, &bob_token] {
        ok(
            TestClient::post(format!(
                "http://localhost/_matrix/client/v3/rooms/{room}/leave"
            ))
            .add_header("Authorization", format!("Bearer {token}"), true)
            .json(&json!({})),
            &service,
        )
        .await;
    }
    assert!(!room::state::user_can_see_events(&bob, &room).await.unwrap());
    for token in [&alice_token, &charlie_token] {
        ok(
            TestClient::post(format!(
                "http://localhost/_matrix/client/v3/rooms/{room}/join"
            ))
            .add_header("Authorization", format!("Bearer {token}"), true)
            .json(&json!({})),
            &service,
        )
        .await;
    }
    let future = sync(&service, &alice_token, None).await;
    let future = future["next_batch"].as_str().unwrap();
    let current = ok(members_request(&room, &alice_token, future), &service).await;
    assert_eq!(
        member(&current, &charlie).unwrap()["content"]["membership"],
        "join"
    );
    let capped = ok(members_request(&room, &bob_token, future), &service).await;
    assert!(member(&capped, &charlie).is_none(), "{capped}");
    assert_eq!(
        member(&capped, &alice).unwrap()["content"]["membership"],
        "leave"
    );
    assert_eq!(
        member(&capped, &bob).unwrap()["content"]["membership"],
        "leave"
    );
    let still_before = ok(members_request(&room, &alice_token, prev), &service).await;
    assert_eq!(
        snapshot(&still_before),
        snapshot(&before_join),
        "an issued p boundary moved with current state"
    );

    // The existing identity and topology token branches keep their semantics.
    let legacy = ok(
        members_request(&room, &alice_token, &bob_join.live_token().to_string()),
        &service,
    )
    .await;
    let historical = ok(
        members_request(&room, &alice_token, &bob_join.historic_token().to_string()),
        &service,
    )
    .await;
    assert_eq!(snapshot(&legacy), snapshot(&historical));
    assert_eq!(
        member(&legacy, &bob).unwrap()["content"]["membership"],
        "join"
    );
    assert!(member(&legacy, &charlie).is_none());
    let q = BatchToken::new_publication_page(
        None,
        Some(next.parse::<BatchToken>().unwrap().stream_ordering()),
        bob_join.depth as i64,
        bob_join.event_sn,
    )
    .to_string();
    assert!(matches!(
        q.parse().unwrap(),
        BatchToken::PublicationPage { .. }
    ));
    let mut rejected = send(members_request(&room, &alice_token, &q), &service).await;
    let body: Value = rejected.take_json().await.unwrap();
    assert_eq!(
        rejected.status_code,
        Some(StatusCode::BAD_REQUEST),
        "{body}"
    );
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_members_initial_sync_prev_batch_preserves_members_point() {
    std::thread::Builder::new()
        .name("members-initial-prev-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(initial_sync_members());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn initial_sync_members() {
    crate::test_database::init();
    let _ = config::CONFIG.set(
        serde_json::from_value(json!({
            "server_name":"members-publication.example", "db":{"url":"unused-test-config"},
            "ip_range_denylist":[], "auto_join_rooms":[]
        }))
        .unwrap(),
    );
    let prefix = format!("members-initial-{}", uuid::Uuid::new_v4().simple());
    let (alice, alice_token) = provision(&format!("{prefix}-alice")).await;
    let (bob, bob_token) = provision(&format!("{prefix}-bob")).await;
    let service = Service::new(crate::routing::root());
    let created = ok(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", format!("Bearer {alice_token}"), true)
            .json(&json!({"room_version":"11", "preset":"public_chat"})),
        &service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    let hello = ok(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/hello"
        ))
        .add_header("Authorization", format!("Bearer {alice_token}"), true)
        .json(&json!({"msgtype":"m.text", "body":"Hello world!"})),
        &service,
    )
    .await;
    // Match Complement TestGetRoomMembersAtPoint: an initial sync with no
    // timeline limit/filter. The limited incremental fixture above is distinct.
    let initial = ok(
        TestClient::get("http://localhost/_matrix/client/v3/sync?timeout=0").add_header(
            "Authorization",
            format!("Bearer {alice_token}"),
            true,
        ),
        &service,
    )
    .await;
    let timeline = &initial["rooms"]["join"][room.as_str()]["timeline"];
    assert_eq!(timeline["limited"], false, "{initial}");
    assert_eq!(timeline["events"][0]["type"], "m.room.create", "{initial}");
    assert_eq!(
        timeline["events"].as_array().unwrap().last().unwrap()["event_id"],
        hello["event_id"],
        "{initial}"
    );
    let previous = timeline["prev_batch"].as_str().unwrap();
    assert!(matches!(
        previous.parse().unwrap(),
        BatchToken::Publication { .. }
    ));
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/join"
        ))
        .add_header("Authorization", format!("Bearer {bob_token}"), true)
        .json(&json!({})),
        &service,
    )
    .await;
    ok(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/hello-back"
        ))
        .add_header("Authorization", format!("Bearer {bob_token}"), true)
        .json(&json!({"msgtype":"m.text", "body":"Hello back"})),
        &service,
    )
    .await;
    let historical = ok(members_request(&room, &alice_token, previous), &service).await;
    assert_eq!(
        historical["chunk"].as_array().unwrap().len(),
        1,
        "{historical}"
    );
    assert_eq!(
        member(&historical, &alice).unwrap()["content"]["membership"],
        "join",
        "{historical}"
    );
    assert!(member(&historical, &bob).is_none(), "{historical}");

    // Positive control: the later actual sync position includes Bob, so the
    // old snapshot cannot pass merely because the join was never published.
    let current = ok(
        TestClient::get("http://localhost/_matrix/client/v3/sync?timeout=0").add_header(
            "Authorization",
            format!("Bearer {alice_token}"),
            true,
        ),
        &service,
    )
    .await;
    let current = ok(
        members_request(&room, &alice_token, current["next_batch"].as_str().unwrap()),
        &service,
    )
    .await;
    assert_eq!(current["chunk"].as_array().unwrap().len(), 2, "{current}");
    assert_eq!(
        member(&current, &bob).unwrap()["content"]["membership"],
        "join",
        "{current}"
    );
}

async fn join_period_frame(room: &OwnedRoomId, state: room::state::CompressedState) -> i64 {
    // These deliberately incomplete snapshots must not reuse a production
    // delta's fingerprint, whose reconstructed full state can be different.
    let hash = crate::utils::hash_keys(
        std::iter::once(b"partial-join-period-fixture".as_slice())
            .chain(state.iter().map(|entry| entry.as_bytes())),
    );
    let frame = room::state::ensure_frame(room, hash).await.unwrap();
    room::state::save_state_delta(
        room,
        frame,
        room::state::StateDiff {
            parent_id: None,
            appended: std::sync::Arc::new(state),
            disposed: std::sync::Arc::new(Default::default()),
        },
    )
    .await
    .unwrap();
    frame
}

async fn join_period_sync(service: &Service, token: &str) -> Value {
    ok(
        TestClient::get("http://localhost/_matrix/client/v3/sync?timeout=0").add_header(
            "Authorization",
            format!("Bearer {token}"),
            true,
        ),
        service,
    )
    .await
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_sync_join_period_uses_only_available_member_proof() {
    std::thread::Builder::new()
        .name("partial-join-period-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(partial_join_period());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn partial_join_period() {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    crate::test_database::init();
    let _ = config::CONFIG.set(
        serde_json::from_value(json!({
            "server_name":"members-publication.example", "db":{"url":"unused-test-config"},
            "ip_range_denylist":[], "auto_join_rooms":[]
        }))
        .unwrap(),
    );
    let prefix = format!("partial-period-{}", uuid::Uuid::new_v4().simple());
    let (alice, token) = provision(&prefix).await;
    let service = Service::new(crate::routing::root());
    let created = ok(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", format!("Bearer {token}"), true)
            .json(&json!({"room_version":"11", "preset":"public_chat"})),
        &service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    let first_join = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        alice.as_str(),
        None,
    )
    .await
    .unwrap();
    ok(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/m.room.member/{alice}"
        ))
        .add_header("Authorization", format!("Bearer {token}"), true)
        .json(&json!({"membership":"join","displayname":"current complete member"})),
        &service,
    )
    .await;
    let current = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        alice.as_str(),
        None,
    )
    .await
    .unwrap();
    assert_ne!(current.event_id, first_join.event_id);
    let own_field = room::state::ensure_field_id(
        &crate::core::events::StateEventType::RoomMember,
        alice.as_str(),
    )
    .await
    .unwrap();

    // A known signed historical membership can have metadata but no fetched
    // payload. It remains an outlier; it grants no room access or publication.
    let (historic, historic_json) = crate::event::PduBuilder {
        event_type: "m.room.member".into(),
        state_key: Some(alice.to_string()),
        content: serde_json::value::to_raw_value(
            &json!({"membership":"join","displayname":"historical proof"}),
        )
        .unwrap(),
        ..Default::default()
    }
    .hash_sign(&alice, &room, &crate::core::RoomVersionId::V11)
    .await
    .unwrap();
    let (historic, _, guard) =
        crate::event::PduBuilder::save_as_outlier(historic, historic_json, &alice)
            .await
            .unwrap();
    drop(guard);
    let stored = data::schema::event_datas::table
        .filter(data::schema::event_datas::event_id.eq(&*historic.event_id))
        .select(data::schema::event_datas::json_data)
        .first::<Value>(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    diesel::delete(
        data::schema::event_datas::table
            .filter(data::schema::event_datas::event_id.eq(&*historic.event_id)),
    )
    .execute(&mut data::connect().await.unwrap())
    .await
    .unwrap();
    let mut own_unknown = room::state::CompressedState::new();
    own_unknown.insert(room::state::CompressedEvent::new(
        own_field,
        historic.event_sn,
    ));
    let own_unknown = join_period_frame(&room, own_unknown).await;
    crate::event::update_before_frame_id(&current.event_id, own_unknown)
        .await
        .unwrap();

    // Actual initial, unfiltered HTTP sync must keep its complete current room.
    // Old strict join-period traversal returns 404 for this unavailable payload.
    let partial = join_period_sync(&service, &token).await;
    assert!(
        partial["rooms"]["join"].get(room.as_str()).is_some(),
        "{partial}"
    );
    assert_eq!(
        room::user::join_sn_for_event(&alice, current.event_id.clone(), current.event_sn)
            .await
            .unwrap(),
        current.event_sn,
        "unknown history may only certify the oldest currently known join"
    );

    // A malformed selected payload is a real decoder/storage failure, not
    // unavailable history. It must still fail the HTTP request.
    let mut restored = data::room::DbEventData {
        event_id: historic.event_id.clone(),
        event_sn: historic.event_sn,
        room_id: room.clone(),
        internal_metadata: None,
        json_data: json!(["invalid stored historical member"]),
        format_version: None,
    };
    restored
        .save_with_conn(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    let mut failure = send(
        TestClient::get("http://localhost/_matrix/client/v3/sync?timeout=0").add_header(
            "Authorization",
            format!("Bearer {token}"),
            true,
        ),
        &service,
    )
    .await;
    let body: Value = failure.take_json().await.unwrap();
    assert_eq!(failure.status_code, Some(StatusCode::BAD_REQUEST), "{body}");
    assert_eq!(body["errcode"], "M_UNKNOWN", "{body}");
    assert!(
        body.get("next_batch").is_none(),
        "failed sync acknowledged a cursor: {body}"
    );
    let decoder_error =
        room::user::join_sn_for_event(&alice, current.event_id.clone(), current.event_sn)
            .await
            .unwrap_err();
    assert!(!decoder_error.is_not_found());
    assert!(
        matches!(&decoder_error, crate::AppError::Internal(message) if message == "invalid pdu in db"),
        "wrong selected-payload failure: {decoder_error:?}"
    );

    // Completing the same proof must extend the joined period immediately.
    // This fails if the provisional current.event_sn was put in JOIN_SN_CACHE.
    restored.json_data = stored;
    restored
        .save_with_conn(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        room::user::join_sn_for_event(&alice, current.event_id.clone(), current.event_sn)
            .await
            .unwrap(),
        historic.event_sn,
        "partial joined-period results must not be cached"
    );
    let completed = join_period_sync(&service, &token).await;
    assert!(
        completed["rooms"]["join"].get(room.as_str()).is_some(),
        "{completed}"
    );

    // Use a new member and a new profile event: neither joined-period proof has
    // been requested, so this cannot pass through the completed Alice cache.
    let (unrelated_user, unrelated_token) = provision(&format!("{prefix}-unrelated")).await;
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/join"
        ))
        .add_header("Authorization", format!("Bearer {unrelated_token}"), true)
        .json(&json!({})),
        &service,
    )
    .await;
    let unrelated_first_join = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        unrelated_user.as_str(),
        None,
    )
    .await
    .unwrap();
    ok(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/m.room.member/{unrelated_user}"
        ))
        .add_header("Authorization", format!("Bearer {unrelated_token}"), true)
        .json(&json!({"membership":"join","displayname":"uncached unrelated proof"})),
        &service,
    )
    .await;
    let unrelated_current = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        unrelated_user.as_str(),
        None,
    )
    .await
    .unwrap();
    assert_ne!(unrelated_current.event_id, unrelated_first_join.event_id);
    let unrelated_before = room::state::get_pdu_before_frame_id(&unrelated_current.event_id)
        .await
        .unwrap();

    // An unrelated unavailable state entry cannot poison this own-member proof.
    let mut before = (*room::state::load_frame_info(unrelated_before)
        .await
        .unwrap()
        .last()
        .unwrap()
        .full_state)
        .clone();
    let other_field =
        room::state::ensure_field_id(&"com.example.unavailable_state".into(), &prefix)
            .await
            .unwrap();
    let unavailable_sn = i64::MAX - 32;
    assert!(
        !diesel::select(diesel::dsl::exists(
            data::schema::event_points::table
                .filter(data::schema::event_points::event_sn.eq(unavailable_sn)),
        ))
        .get_result::<bool>(&mut data::connect().await.unwrap())
        .await
        .unwrap()
    );
    before.insert(room::state::CompressedEvent::new(
        other_field,
        unavailable_sn,
    ));
    let unrelated = join_period_frame(&room, before).await;
    assert!(
        room::state::get_full_state_ids(unrelated)
            .await
            .unwrap_err()
            .is_not_found()
    );
    crate::event::update_before_frame_id(&unrelated_current.event_id, unrelated)
        .await
        .unwrap();
    let full = join_period_sync(&service, &unrelated_token).await;
    assert!(full["rooms"]["join"].get(room.as_str()).is_some(), "{full}");
    assert_eq!(
        room::user::join_sn_for_event(
            &unrelated_user,
            unrelated_current.event_id.clone(),
            unrelated_current.event_sn,
        )
        .await
        .unwrap(),
        unrelated_first_join.event_sn,
        "unrelated missing state must not affect the own-member joined period"
    );

    // A fresh current member avoids the completed proof's cache. Cycles and
    // actual SQL failures must still propagate instead of becoming partial.
    ok(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/m.room.member/{alice}"
        ))
        .add_header("Authorization", format!("Bearer {token}"), true)
        .json(&json!({"membership":"join","displayname":"fresh proof"})),
        &service,
    )
    .await;
    let fresh = room::get_state(
        &room,
        &crate::core::events::StateEventType::RoomMember,
        alice.as_str(),
        None,
    )
    .await
    .unwrap();
    let fresh_before = room::state::get_pdu_before_frame_id(&fresh.event_id)
        .await
        .unwrap();
    let mut cycle = room::state::CompressedState::new();
    cycle.insert(room::state::CompressedEvent::new(own_field, fresh.event_sn));
    let cycle = join_period_frame(&room, cycle).await;
    crate::event::update_before_frame_id(&fresh.event_id, cycle)
        .await
        .unwrap();
    let cycle_error = room::user::join_sn_for_event(&alice, fresh.event_id.clone(), fresh.event_sn)
        .await
        .unwrap_err();
    assert!(
        !cycle_error.is_not_found(),
        "cycle was hidden as partial history"
    );
    crate::event::update_before_frame_id(&fresh.event_id, fresh_before)
        .await
        .unwrap();

    diesel::sql_query(
        "ALTER TABLE event_datas RENAME COLUMN json_data TO unavailable_join_period_json",
    )
    .execute(&mut data::connect().await.unwrap())
    .await
    .unwrap();
    let backend_error =
        room::user::join_sn_for_event(&alice, fresh.event_id.clone(), fresh.event_sn).await;
    diesel::sql_query(
        "ALTER TABLE event_datas RENAME COLUMN unavailable_join_period_json TO json_data",
    )
    .execute(&mut data::connect().await.unwrap())
    .await
    .unwrap();
    let backend_error = backend_error.unwrap_err();
    assert!(
        !backend_error.is_not_found(),
        "SQL failure was hidden as partial history"
    );
    let recovered = join_period_sync(&service, &token).await;
    assert!(
        recovered["rooms"]["join"].get(room.as_str()).is_some(),
        "{recovered}"
    );
}
