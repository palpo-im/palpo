//! Real room lifecycle, durable push work and a loopback HTTP gateway.
use std::sync::{Arc, Mutex};

use salvo::conn::Acceptor;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

const LOCAL: &str = "push-authorization.example";

#[derive(Clone)]
struct Gateway(Arc<Mutex<Vec<Value>>>);

#[async_trait::async_trait]
impl Handler for Gateway {
    async fn handle(
        &self,
        req: &mut Request,
        _depot: &mut Depot,
        res: &mut Response,
        _ctrl: &mut FlowCtrl,
    ) {
        let body = serde_json::from_slice(req.payload().await.unwrap()).unwrap();
        self.0.lock().unwrap().push(body);
        res.render(Json(json!({"rejected":[]})));
    }
}

fn user(name: &str) -> OwnedUserId {
    UserId::parse(format!("@{name}:{LOCAL}")).unwrap()
}

fn kind(name: &str) -> OutgoingKind {
    OutgoingKind::Push(user(name), format!("push-{name}"))
}

async fn request(service: &Service, builder: RequestBuilder, name: &str) -> Value {
    let builder = builder.add_header("Authorization", format!("Bearer {name}"), true);
    let router = service.router();
    let mut response = tokio::spawn(async move { builder.send(router).await })
        .await
        .unwrap();
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

async fn membership_event(room: &RoomId, name: &str) -> OwnedEventId {
    room_users::table
        .filter(room_users::room_id.eq(room))
        .filter(room_users::user_id.eq(user(name)))
        .select(room_users::event_id)
        .first(&mut data::db())
        .await
        .unwrap()
}

async fn assert_queued(name: &str, event: &EventId, expected: bool) {
    let queued = queued_requests(&kind(name), 100).await.unwrap();
    assert_eq!(
        queued
            .iter()
            .any(|(_, value)| matches!(value, SendingEventType::Pdu(id) if id == event)),
        expected,
        "{name} {event}"
    );
}

async fn deliver(name: &str, event: &EventId) {
    send_events(kind(name), vec![SendingEventType::Pdu(event.to_owned())])
        .await
        .unwrap();
}

async fn clear_queue() {
    diesel::delete(outgoing_requests::table.filter(outgoing_requests::kind.eq("push")))
        .execute(&mut data::db())
        .await
        .unwrap();
}

async fn message(service: &Service, room: &RoomId, id: &str, body: &str) -> OwnedEventId {
    let response = request(
        service,
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/{id}"
        ))
        .json(&json!({"msgtype":"m.text", "body":body})),
        "alice",
    )
    .await;
    EventId::parse(response["event_id"].as_str().unwrap()).unwrap()
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn http_push_authorization_tracks_membership_and_current_invitation() {
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
                "server_name":LOCAL, "db":{"url":"unused"}, "ip_range_denylist":[],
                "max_concurrent_requests":1,
                "rc_message":{"per_second":10000,"burst":10000},
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false},
            }))
            .unwrap(),
        )
        .unwrap();
    // Retain the real wakeup channel but drive delivery explicitly so removal
    // can be placed deterministically between durable enqueue and HTTP send.
    let _guard = guard::init();
    for name in ["alice", "bob", "carol", "dave"] {
        let user_id = user(name);
        data::user::create_user(&data::user::NewDbUser {
            id: user_id.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: name.to_owned(),
            server_name: user_id.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        // Registered users have a global profile; the real local ban handler
        // reads it when constructing the membership event.
        data::user::ensure_profile_exists(&user_id).await.unwrap();
        data::user::device::create_device(&user_id, &OwnedDeviceId::from("D"), name, None, None)
            .await
            .unwrap();
    }
    let gateway = Gateway(Arc::new(Mutex::new(Vec::new())));
    let listener = TcpListener::new("127.0.0.1:0").bind().await;
    let address = listener.holdings()[0]
        .local_addr
        .clone()
        .into_std()
        .unwrap();
    let peer = tokio::spawn(
        Server::new(listener).serve(Router::with_path("notify").post(gateway.clone())),
    );
    let service = Service::new(crate::routing::root());
    for name in ["bob", "carol", "dave"] {
        request(
            &service,
            TestClient::post("http://localhost/_matrix/client/v3/pushers/set").json(&json!({
                "kind":"http", "app_id":"authorization-test", "app_display_name":"test",
                "device_display_name":"test", "pushkey":format!("push-{name}"), "lang":"en",
                "data":{"url":format!("http://{address}/notify")},
            })),
            name,
        )
        .await;
    }
    let created = request(&service, TestClient::post("http://localhost/_matrix/client/v3/createRoom").json(&json!({
        "preset":"private_chat", "room_version":"11", "invite":[user("bob"),user("carol"),user("dave")],
    })), "alice").await;
    let room = RoomId::parse(created["room_id"].as_str().unwrap()).unwrap();
    assert_eq!(
        crate::room::get_our_real_users(&room).await.unwrap(),
        vec![user("alice")]
    );

    // A new invitation must be delivered after its membership is published.
    for name in ["bob", "carol", "dave"] {
        let invite = membership_event(&room, name).await;
        assert_queued(name, &invite, true).await;
        deliver(name, &invite).await;
        let received = gateway.0.lock().unwrap().last().unwrap().clone();
        assert_eq!(received["notification"]["event_id"], invite.as_str());
        assert_eq!(received["notification"]["content"]["membership"], "invite");
        assert_eq!(received["notification"]["user_is_target"], true);
        request(
            &service,
            TestClient::post(format!("http://localhost/_matrix/client/v3/join/{room}"))
                .json(&json!({})),
            name,
        )
        .await;
    }
    clear_queue().await;
    let normal = message(&service, &room, "normal", "visible to current members").await;
    for name in ["bob", "carol", "dave"] {
        assert_queued(name, &normal, true).await;
        deliver(name, &normal).await;
        let received = gateway.0.lock().unwrap().last().unwrap().clone();
        assert_eq!(
            received["notification"]["content"]["body"],
            "visible to current members"
        );
    }
    clear_queue().await;
    let delayed = message(&service, &room, "before-removal", "queued while joined").await;
    for name in ["bob", "carol", "dave"] {
        assert_queued(name, &delayed, true).await;
    }
    request(
        &service,
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/leave"
        ))
        .json(&json!({})),
        "bob",
    )
    .await;
    for (action, name) in [("kick", "carol"), ("ban", "dave")] {
        request(
            &service,
            TestClient::post(format!(
                "http://localhost/_matrix/client/v3/rooms/{room}/{action}"
            ))
            .json(&json!({"user_id":user(name)})),
            "alice",
        )
        .await;
    }
    assert_eq!(
        crate::room::get_our_real_users(&room).await.unwrap(),
        vec![user("alice")]
    );
    // Previously visible history is not a grant to continue live push delivery.
    assert!(
        crate::room::state::user_can_see_event(&user("bob"), &delayed)
            .await
            .unwrap()
    );
    let received = gateway.0.lock().unwrap().len();
    for name in ["bob", "carol", "dave"] {
        deliver(name, &delayed).await;
    }
    assert_eq!(gateway.0.lock().unwrap().len(), received);
    clear_queue().await;

    let after = message(
        &service,
        &room,
        "after-removal",
        "private after all removals",
    )
    .await;
    for name in ["bob", "carol", "dave"] {
        assert_queued(name, &after, false).await;
        deliver(name, &after).await;
    }
    assert_eq!(gateway.0.lock().unwrap().len(), received);
    let actions = event_push_actions::table
        .filter(event_push_actions::event_id.eq(&after))
        .filter(event_push_actions::user_id.eq_any([user("bob"), user("carol"), user("dave")]))
        .count()
        .get_result::<i64>(&mut data::db())
        .await
        .unwrap();
    assert_eq!(actions, 0);

    // Mere invited membership never permits ordinary messages. An invitation
    // retry must also stop when its recipient declines it.
    request(
        &service,
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/invite"
        ))
        .json(&json!({"user_id":user("bob")})),
        "alice",
    )
    .await;
    let invite = membership_event(&room, "bob").await;
    assert_queued("bob", &invite, true).await;
    let while_invited = message(&service, &room, "while-invited", "not visible to invitees").await;
    assert_queued("bob", &while_invited, false).await;
    deliver("bob", &while_invited).await;
    assert_eq!(gateway.0.lock().unwrap().len(), received);
    request(
        &service,
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/leave"
        ))
        .json(&json!({})),
        "bob",
    )
    .await;
    deliver("bob", &invite).await;
    assert_eq!(gateway.0.lock().unwrap().len(), received);
    request(
        &service,
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/invite"
        ))
        .json(&json!({"user_id":user("bob")})),
        "alice",
    )
    .await;
    let replacement = membership_event(&room, "bob").await;
    assert_ne!(replacement, invite);
    deliver("bob", &invite).await;
    assert_eq!(gateway.0.lock().unwrap().len(), received);
    deliver("bob", &replacement).await;
    assert_eq!(gateway.0.lock().unwrap().len(), received + 1);
    peer.abort();
}
