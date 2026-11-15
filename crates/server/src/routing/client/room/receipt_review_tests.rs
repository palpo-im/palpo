//! Real client routes and PostgreSQL, isolated per exact test name.
use diesel::prelude::*;
use diesel_async::SimpleAsyncConnection;
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{
    OwnedDeviceId, OwnedEventId, OwnedRoomId, OwnedUserId, RoomId, UnixMillis, UserId,
};
use crate::data::schema::*;
use crate::{config, data};

async fn request(service: &Service, token: &str, request: RequestBuilder) -> (StatusCode, Value) {
    let router = service.router();
    let request = request.add_header("authorization", format!("Bearer {token}"), true);
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        (
            response.status_code.unwrap(),
            response.take_json().await.unwrap(),
        )
    })
    .await
    .unwrap()
}

async fn ok(service: &Service, token: &str, request_body: RequestBuilder) -> Value {
    let (status, body) = request(service, token, request_body).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn user(name: &str) -> OwnedUserId {
    let user = UserId::parse(format!("@{name}:{}", config::server_name())).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: name.into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&user).await.unwrap();
    data::user::device::create_device(&user, &OwnedDeviceId::from("D"), name, None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&user).await.unwrap();
    user
}

async fn message(
    service: &Service,
    room: &RoomId,
    txn: &str,
    relation: Option<&OwnedEventId>,
) -> OwnedEventId {
    let mut content = json!({"msgtype":"m.text", "body":txn});
    if let Some(root) = relation {
        content["m.relates_to"] = json!({"rel_type":"m.thread", "event_id":root,
            "is_falling_back":true, "m.in_reply_to":{"event_id":root}});
    }
    let result = ok(
        service,
        "owner",
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/{txn}"
        ))
        .json(&content),
    )
    .await;
    result["event_id"].as_str().unwrap().try_into().unwrap()
}

fn endpoint(room: &RoomId, suffix: &str) -> String {
    format!("http://localhost/_matrix/client/v3/rooms/{room}/{suffix}")
}

async fn v5_sync(
    service: &Service,
    token: &str,
    room: &RoomId,
    conn: &str,
    pos: Option<&str>,
) -> Value {
    let mut url = url::Url::parse(
        "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync",
    )
    .unwrap();
    url.query_pairs_mut().append_pair("timeout", "0");
    if let Some(pos) = pos {
        url.query_pairs_mut().append_pair("pos", pos);
    }
    ok(
        service,
        token,
        TestClient::post(url.as_str()).json(&json!({
            "conn_id":conn,
            "room_subscriptions":{(room.as_str()):{"timeline_limit":0,"required_state":[]}},
            "extensions":{"receipts":{"enabled":true,"rooms":["*"]}}
        })),
    )
    .await
}

fn private_scopes(content: &Value, user: &UserId) -> Vec<String> {
    let mut scopes: Vec<_> = content
        .as_object()
        .expect("receipt content must be present")
        .values()
        .filter_map(|receipts| receipts["m.read.private"].get(user.as_str()))
        .map(|receipt| receipt["thread_id"].as_str().unwrap_or("").to_owned())
        .collect();
    scopes.sort();
    scopes
}

async fn actions(user: &OwnedUserId, room: &OwnedRoomId) -> Vec<OwnedEventId> {
    event_push_actions::table
        .filter(event_push_actions::user_id.eq(user))
        .filter(event_push_actions::room_id.eq(room))
        .order(event_push_actions::event_id)
        .select(event_push_actions::event_id)
        .load(&mut data::db())
        .await
        .unwrap()
}

async fn snapshot() -> (i64, i64, Vec<(i64, i64, Value)>, i64) {
    (
        event_receipts::table
            .count()
            .get_result(&mut data::db())
            .await
            .unwrap(),
        outgoing_requests::table
            .count()
            .get_result(&mut data::db())
            .await
            .unwrap(),
        user_datas::table
            .order(user_datas::id)
            .select((user_datas::id, user_datas::occur_sn, user_datas::json_data))
            .load(&mut data::db())
            .await
            .unwrap(),
        event_push_actions::table
            .count()
            .get_result(&mut data::db())
            .await
            .unwrap(),
    )
}

async fn refused(service: &Service, token: &str, request_body: RequestBuilder) {
    let before = snapshot().await;
    let (status, body) = request(service, token, request_body).await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::BAD_REQUEST),
        "{status}: {body}"
    );
    assert_eq!(
        snapshot().await,
        before,
        "refusal must not write markers, receipts or broadcast"
    );
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_receipts_preserve_notifications_scopes_and_authorization() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn run() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"receipt-review.example", "db":{"url":"unused"},
                "auto_join_rooms":[], "federation":{"enable":true},
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false},
                "rc_message":{"per_second":10000,"burst":1000}
            }))
            .unwrap(),
        )
        .unwrap();
    // Exercise the durable raw EDU queue while keeping the peer controlled and offline.
    let (wakeups, _receiver) = tokio::sync::mpsc::channel(4096);
    crate::sending::MPSC_SENDER.set(wakeups).unwrap();
    let owner = user("owner").await;
    let reader = user("reader").await;
    user("outsider").await;
    let service = Service::new(crate::routing::root());
    let created = ok(
        &service,
        "owner",
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .json(&json!({"room_version":"11","preset":"private_chat","invite":[reader]})),
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    ok(
        &service,
        "owner",
        TestClient::put(endpoint(&room, "state/m.room.history_visibility"))
            .json(&json!({"history_visibility":"joined"})),
    )
    .await;
    let hidden = message(&service, &room, "before-reader-join", None).await;
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, "join")).json(&json!({})),
    )
    .await;
    let a = message(&service, &room, "A", None).await;
    let b = message(&service, &room, "B", None).await;
    let c = message(&service, &room, "C", None).await;
    // Deterministic notification fixture on real, authorized timeline events.
    diesel::update(
        event_push_actions::table
            .filter(event_push_actions::user_id.eq(&reader))
            .filter(event_push_actions::event_id.eq_any([&a, &b, &c])),
    )
    .set((
        event_push_actions::notify.eq(true),
        event_push_actions::unread.eq(true),
    ))
    .execute(&mut data::db())
    .await
    .unwrap();
    crate::room::push_action::refresh_notify_summary(&reader, &room)
        .await
        .unwrap();
    for path in [
        "read_markers".to_owned(),
        format!("receipt/m.fully_read/{a}"),
    ] {
        let body = if path == "read_markers" {
            json!({"m.fully_read":a})
        } else {
            json!({})
        };
        let before = actions(&reader, &room).await;
        ok(
            &service,
            "reader",
            TestClient::post(endpoint(&room, &path)).json(&body),
        )
        .await;
        assert_eq!(
            actions(&reader, &room).await,
            before,
            "fully-read must not clear notifications"
        );
    }
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, "read_markers"))
            .json(&json!({"m.fully_read":a,"m.read":a})),
    )
    .await;
    let remaining = actions(&reader, &room).await;
    assert!(remaining.contains(&b) && remaining.contains(&c));
    assert!(!remaining.contains(&a));
    assert!(
        crate::room::user::notify_summary(&reader, &room)
            .await
            .unwrap()
            .notification_count
            >= 2
    );

    let root1 = message(&service, &room, "root-one", None).await;
    let root2 = message(&service, &room, "root-two", None).await;
    let reply1 = message(&service, &room, "reply-one", Some(&root1)).await;
    let reply2 = message(&service, &room, "reply-two", Some(&root2)).await;
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{root1}")))
            .json(&json!({"thread_id":root1})),
    )
    .await;
    let remaining = actions(&reader, &room).await;
    assert!(
        remaining.contains(&root1) && remaining.contains(&reply1) && remaining.contains(&reply2)
    );
    let mut indirect = Vec::new();
    for (txn, ty, relation) in [
        (
            "reference-to-reply",
            "m.room.message",
            json!({"rel_type":"m.reference","event_id":reply1}),
        ),
        (
            "reaction-to-reply",
            "m.reaction",
            json!({"rel_type":"m.annotation","event_id":reply1,"key":"+1"}),
        ),
        (
            "rich-reply-to-reply",
            "m.room.message",
            json!({"m.in_reply_to":{"event_id":reply1}}),
        ),
    ] {
        let response = ok(
            &service,
            "owner",
            TestClient::put(endpoint(&room, &format!("send/{ty}/{txn}")))
                .json(&json!({"msgtype":"m.text","body":txn,"m.relates_to":relation})),
        )
        .await;
        let id: OwnedEventId = response["event_id"].as_str().unwrap().try_into().unwrap();
        let projected = event_points::table
            .find(&id)
            .select(event_points::thread_id)
            .first::<Option<OwnedEventId>>(&mut data::db())
            .await
            .unwrap();
        assert_eq!(
            projected,
            Some(root1.clone()),
            "indirect notification scope must agree with receipt validation"
        );
        indirect.push(id);
    }
    assert_eq!(
        event_points::table
            .find(&root1)
            .select(event_points::thread_id)
            .first::<Option<OwnedEventId>>(&mut data::db())
            .await
            .unwrap(),
        None,
        "thread roots remain main"
    );
    refused(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{}", indirect[1])))
            .json(&json!({"thread_id":reply1})),
    )
    .await;
    let main = message(&service, &room, "main-after-threads", None).await;
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{main}")))
            .json(&json!({"thread_id":"main"})),
    )
    .await;
    let remaining = actions(&reader, &room).await;
    assert!(remaining.contains(&reply1) && remaining.contains(&reply2));
    assert!(
        indirect.iter().all(|id| remaining.contains(id)),
        "main receipt cleared indirect thread actions"
    );
    assert!(!remaining.contains(&main));
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{reply1}")))
            .json(&json!({"thread_id":root1})),
    )
    .await;
    let remaining = actions(&reader, &room).await;
    assert!(!remaining.contains(&reply1) && remaining.contains(&reply2));
    for id in &indirect {
        ok(
            &service,
            "reader",
            TestClient::post(endpoint(&room, &format!("receipt/m.read/{id}")))
                .json(&json!({"thread_id":root1})),
        )
        .await;
        assert!(
            !actions(&reader, &room).await.contains(id),
            "thread receipt did not clear an indirect relation"
        );
    }

    // Complement's TestThreadedReceipts acknowledges an annotation of a
    // reference to the root. That is a valid receipt target even though neither
    // event follows an m.thread edge and both retain main notification scope.
    let tail = message(
        &service,
        &room,
        "thread-before-root-reference",
        Some(&root1),
    )
    .await;
    let mut parent = root1.clone();
    let mut main_relations = Vec::new();
    for (txn, ty, rel_type) in [
        ("reference-to-root", "m.room.message", "m.reference"),
        ("annotation-to-reference", "m.reaction", "m.annotation"),
    ] {
        let response = ok(
            &service,
            "owner",
            TestClient::put(endpoint(&room, &format!("send/{ty}/{txn}"))).json(&json!({
                "msgtype":"m.text", "body":txn,
                "m.relates_to":{"rel_type":rel_type,"event_id":parent,"key":"+1"}
            })),
        )
        .await;
        parent = response["event_id"].as_str().unwrap().try_into().unwrap();
        assert_eq!(
            event_points::table
                .find(&parent)
                .select(event_points::thread_id)
                .first::<Option<OwnedEventId>>(&mut data::db())
                .await
                .unwrap(),
            None,
            "a reference to the root must retain its main notification scope"
        );
        main_relations.push(parent.clone());
    }
    refused(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{parent}")))
            .json(&json!({"thread_id":root2})),
    )
    .await;
    refused(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{parent}")))
            .json(&json!({"thread_id":main_relations[0]})),
    )
    .await;
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{parent}")))
            .json(&json!({"thread_id":root1})),
    )
    .await;
    let remaining = actions(&reader, &room).await;
    assert!(!remaining.contains(&tail) && remaining.contains(&reply2));
    assert!(main_relations.iter().all(|id| remaining.contains(id)));
    let main = message(&service, &room, "main-after-root-reference", None).await;
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{main}")))
            .json(&json!({"thread_id":"main"})),
    )
    .await;

    let initial = ok(
        &service,
        "reader",
        TestClient::get("http://localhost/_matrix/client/v3/sync"),
    )
    .await;
    // Establish this connection before the private writes and retain its real
    // position. Incremental verification must not reset the connection first.
    let v5_initial = v5_sync(&service, "reader", &room, "receipt-inc", None).await;
    let v5_pos = v5_initial["pos"].as_str().unwrap().to_owned();
    let v5_owner_initial = v5_sync(&service, "owner", &room, "receipt-own-inc", None).await;
    let v5_owner_pos = v5_owner_initial["pos"].as_str().unwrap().to_owned();
    for (event, scope) in [
        (&reply1, Some(root1.as_str())),
        (&reply2, Some(root2.as_str())),
        (&main, Some("main")),
        (&a, None),
    ] {
        let body = scope
            .map(|scope| json!({"thread_id":scope}))
            .unwrap_or_else(|| json!({}));
        ok(
            &service,
            "reader",
            TestClient::post(endpoint(&room, &format!("receipt/m.read.private/{event}")))
                .json(&body),
        )
        .await;
    }
    let mut expected_private = vec![
        "".to_owned(),
        "main".to_owned(),
        root1.to_string(),
        root2.to_string(),
    ];
    expected_private.sort();
    let v5_incremental = v5_sync(&service, "reader", &room, "receipt-inc", Some(&v5_pos)).await;
    assert_eq!(
        private_scopes(
            &v5_incremental["extensions"]["receipts"]["rooms"][room.as_str()]["content"],
            &reader
        ),
        expected_private,
        "incremental v5 lost private receipt scopes: {v5_incremental}"
    );
    let v5_fresh = v5_sync(&service, "reader", &room, "receipt-init", None).await;
    assert_eq!(
        private_scopes(
            &v5_fresh["extensions"]["receipts"]["rooms"][room.as_str()]["content"],
            &reader
        ),
        expected_private,
        "initial v5 lost private receipt scopes: {v5_fresh}"
    );
    let v5_owner_incremental = v5_sync(
        &service,
        "owner",
        &room,
        "receipt-own-inc",
        Some(&v5_owner_pos),
    )
    .await;
    assert!(
        !serde_json::to_string(&v5_owner_incremental)
            .unwrap()
            .contains("\"m.read.private\""),
        "another user received incremental private receipts through v5: {v5_owner_incremental}"
    );
    let v5_owner = v5_sync(&service, "owner", &room, "receipt-owner", None).await;
    assert!(
        private_scopes(
            &v5_owner["extensions"]["receipts"]["rooms"][room.as_str()]["content"],
            &reader
        )
        .is_empty(),
        "another user received private receipts through v5: {v5_owner}"
    );
    for since in [None, initial["next_batch"].as_str()] {
        let url = since
            .map(|token| format!("http://localhost/_matrix/client/v3/sync?since={token}"))
            .unwrap_or_else(|| "http://localhost/_matrix/client/v3/sync".to_owned());
        let sync = ok(&service, "reader", TestClient::get(url)).await;
        let mut scopes = Vec::new();
        for event in sync["rooms"]["join"][room.as_str()]["ephemeral"]["events"]
            .as_array()
            .unwrap()
        {
            if event["type"] == "m.receipt" {
                for receipts in event["content"].as_object().unwrap().values() {
                    if let Some(receipt) = receipts["m.read.private"].get(reader.as_str()) {
                        scopes.push(receipt["thread_id"].as_str().unwrap_or("").to_owned());
                    }
                }
            }
        }
        scopes.sort();
        let mut expected = vec![
            "".to_owned(),
            "main".to_owned(),
            root1.to_string(),
            root2.to_string(),
        ];
        expected.sort();
        assert_eq!(scopes, expected, "{sync}");
    }
    assert!(
        data::room::receipt::read_receipts(&room, 0, Some(&owner))
            .await
            .unwrap()
            .iter()
            .all(|(_, content)| content.values().all(|receipts| !receipts
                .contains_key(&crate::core::events::receipt::ReceiptType::ReadPrivate)))
    );
    let owner_sync = ok(
        &service,
        "owner",
        TestClient::get("http://localhost/_matrix/client/v3/sync"),
    )
    .await;
    assert!(
        !serde_json::to_string(&owner_sync)
            .unwrap()
            .contains("\"m.read.private\"")
    );

    let peer = crate::core::ServerName::parse("peer.receipt.example").unwrap();
    assert!(data::room::add_joined_server(&room, &peer).await.unwrap());
    for (event, scope) in [(&reply1, root1.as_str()), (&main, "main")] {
        ok(
            &service,
            "reader",
            TestClient::post(endpoint(&room, &format!("receipt/m.read/{event}")))
                .json(&json!({"thread_id":scope})),
        )
        .await;
    }
    let raw = outgoing_requests::table
        .filter(outgoing_requests::edu_json.is_not_null())
        .order(outgoing_requests::id)
        .select(outgoing_requests::edu_json)
        .load::<Option<Vec<u8>>>(&mut data::db())
        .await
        .unwrap();
    let scopes: Vec<_> = raw
        .into_iter()
        .flatten()
        .filter_map(|bytes| {
            let edu: Value = serde_json::from_slice(&bytes).unwrap();
            (edu["edu_type"] == "m.receipt").then(|| {
                edu["content"][room.as_str()]["m.read"][reader.as_str()]["data"]["thread_id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
        })
        .collect();
    assert_eq!(scopes, vec![root1.to_string(), "main".to_owned()]);
    let queued_before: i64 = outgoing_requests::table
        .count()
        .get_result(&mut data::db())
        .await
        .unwrap();
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read.private/{reply1}")))
            .json(&json!({"thread_id":root1})),
    )
    .await;
    let queued_after: i64 = outgoing_requests::table
        .count()
        .get_result(&mut data::db())
        .await
        .unwrap();
    assert_eq!(
        queued_before, queued_after,
        "private receipt must not enter the federation queue"
    );

    let other = ok(
        &service,
        "owner",
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .json(&json!({"room_version":"11","preset":"private_chat"})),
    )
    .await;
    let other_room: OwnedRoomId = other["room_id"].as_str().unwrap().try_into().unwrap();
    let foreign = message(&service, &other_room, "foreign", None).await;
    for ty in ["m.read", "m.read.private", "m.fully_read"] {
        refused(
            &service,
            "outsider",
            TestClient::post(endpoint(&room, &format!("receipt/{ty}/{a}"))).json(&json!({})),
        )
        .await;
        refused(
            &service,
            "reader",
            TestClient::post(endpoint(&room, &format!("receipt/{ty}/{foreign}"))).json(&json!({})),
        )
        .await;
        refused(
            &service,
            "reader",
            TestClient::post(endpoint(&room, &format!("receipt/{ty}/{hidden}"))).json(&json!({})),
        )
        .await;
        refused(
            &service,
            "reader",
            TestClient::post(endpoint(&room, "read_markers")).json(&json!({(ty):foreign})),
        )
        .await;
        refused(
            &service,
            "outsider",
            TestClient::post(endpoint(&room, "read_markers")).json(&json!({(ty):a})),
        )
        .await;
    }
    refused(
        &service,
        "reader",
        TestClient::post(endpoint(&room, "read_markers"))
            .json(&json!({"m.fully_read":b,"m.read":foreign})),
    )
    .await;
    refused(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{main}")))
            .json(&json!({"thread_id":root1})),
    )
    .await;
    refused(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{main}")))
            .json(&json!({"thread_id":"$unknown-receipt-thread"})),
    )
    .await;
    // Traditional receipts still acknowledge every older thread.
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{main}"))).json(&json!({})),
    )
    .await;
    assert!(!actions(&reader, &room).await.contains(&reply2));
    // A summary failure rolls back the watermark and actions together. This
    // tests the route transaction, rather than separately calling its setters.
    let fault_event = message(&service, &room, "receipt-transaction-failure", None).await;
    diesel::update(
        event_push_actions::table
            .filter(event_push_actions::user_id.eq(&reader))
            .filter(event_push_actions::event_id.eq(&fault_event)),
    )
    .set(event_push_actions::notify.eq(true))
    .execute(&mut data::db())
    .await
    .unwrap();
    crate::room::push_action::refresh_notify_summary(&reader, &room)
        .await
        .unwrap();
    data::checkout().await.unwrap().batch_execute("CREATE FUNCTION fail_receipt_summary() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected receipt summary failure'; END $$; CREATE TRIGGER fail_receipt_summary BEFORE INSERT OR UPDATE OR DELETE ON event_push_summaries FOR EACH ROW EXECUTE FUNCTION fail_receipt_summary();").await.unwrap();
    for ty in ["m.read", "m.read.private"] {
        let before = snapshot().await;
        let (status, body) = request(
            &service,
            "reader",
            TestClient::post(endpoint(&room, &format!("receipt/{ty}/{fault_event}")))
                .json(&json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
        assert_eq!(
            snapshot().await,
            before,
            "failed summary committed a receipt or removed actions"
        );
        assert!(actions(&reader, &room).await.contains(&fault_event));
    }
    data::checkout().await.unwrap().batch_execute("DROP TRIGGER fail_receipt_summary ON event_push_summaries; DROP FUNCTION fail_receipt_summary(); CREATE FUNCTION fail_receipt_queue() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected receipt queue failure'; END $$; CREATE TRIGGER fail_receipt_queue BEFORE INSERT ON outgoing_requests FOR EACH ROW EXECUTE FUNCTION fail_receipt_queue();").await.unwrap();
    let receipts_before: i64 = event_receipts::table
        .count()
        .get_result(&mut data::db())
        .await
        .unwrap();
    let (status, body) = request(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{fault_event}")))
            .json(&json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let receipts_after: i64 = event_receipts::table
        .count()
        .get_result(&mut data::db())
        .await
        .unwrap();
    assert_eq!(
        receipts_after,
        receipts_before + 1,
        "local read commit should precede broadcast"
    );
    assert!(!actions(&reader, &room).await.contains(&fault_event));
    assert_eq!(
        crate::room::user::notify_summary(&reader, &room)
            .await
            .unwrap()
            .all_notification_count(),
        0
    );
    data::checkout().await.unwrap().batch_execute("DROP TRIGGER fail_receipt_queue ON outgoing_requests; DROP FUNCTION fail_receipt_queue();").await.unwrap();
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{fault_event}")))
            .json(&json!({})),
    )
    .await;
    ok(
        &service,
        "reader",
        TestClient::post(endpoint(&room, "leave")).json(&json!({})),
    )
    .await;
    refused(
        &service,
        "reader",
        TestClient::post(endpoint(&room, "read_markers")).json(&json!({"m.fully_read":a})),
    )
    .await;
    refused(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read/{a}"))).json(&json!({})),
    )
    .await;
    ok(
        &service,
        "owner",
        TestClient::post(endpoint(&room, "ban")).json(&json!({"user_id":reader})),
    )
    .await;
    refused(
        &service,
        "reader",
        TestClient::post(endpoint(&room, &format!("receipt/m.read.private/{a}"))).json(&json!({})),
    )
    .await;
}
