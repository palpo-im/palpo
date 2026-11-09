//! HTTP regression for the state API's membership snapshot boundary.
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedRoomId, OwnedUserId, UnixMillis, UserId};
use crate::{config, data};

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

fn authed_get(url: String, token: &str) -> RequestBuilder {
    TestClient::get(url).add_header("Authorization", format!("Bearer {token}"), true)
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
    body: Value,
) {
    ok(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/{ty}/{key}"
        ))
        .add_header("Authorization", format!("Bearer {token}"), true)
        .json(&body),
        service,
    )
    .await;
}

fn state_content<'a>(body: &'a Value, ty: &str, key: &str) -> &'a Value {
    &body
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["type"] == ty && event["state_key"] == key)
        .unwrap_or_else(|| panic!("missing {ty}/{key}: {body}"))["content"]
}

async fn assert_snapshot(
    service: &Service,
    room: &OwnedRoomId,
    token: &str,
    owner: &OwnedUserId,
    label: &str,
) {
    let base = format!("http://localhost/_matrix/client/v3/rooms/{room}/state");
    let all = ok(authed_get(base.clone(), token), service).await;
    for (ty, key, expected) in [
        ("m.room.topic", "", json!({"topic":label})),
        ("org.palpo.snapshot", "key", json!({"value":label})),
        (
            "m.room.member",
            owner.as_str(),
            json!({"membership":"join", "displayname":label}),
        ),
    ] {
        assert_eq!(state_content(&all, ty, key), &expected, "{all}");
        let single = ok(authed_get(format!("{base}/{ty}/{key}"), token), service).await;
        assert_eq!(single, expected);
        if key.is_empty() {
            let empty = ok(authed_get(format!("{base}/{ty}"), token), service).await;
            assert_eq!(empty, expected);
        }
    }
}

async fn update_snapshot(
    service: &Service,
    room: &OwnedRoomId,
    token: &str,
    owner: &OwnedUserId,
    label: &str,
) {
    put_state(
        service,
        room,
        token,
        "m.room.topic",
        "",
        json!({"topic":label}),
    )
    .await;
    put_state(
        service,
        room,
        token,
        "org.palpo.snapshot",
        "key",
        json!({"value":label}),
    )
    .await;
    put_state(
        service,
        room,
        token,
        "m.room.member",
        owner.as_str(),
        json!({"membership":"join", "displayname":label}),
    )
    .await;
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn review_state_gets_share_departure_snapshot_and_rejoin_boundary() {
    std::thread::Builder::new()
        .name("state-snapshot-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(state_snapshots())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn state_snapshots() {
    crate::test_database::init();
    let _ = config::CONFIG.set(
        serde_json::from_value(json!({
            "server_name":"state-snapshot.example", "db":{"url":"unused-test-config"},
            "ip_range_denylist":[], "auto_join_rooms":[]
        }))
        .unwrap(),
    );
    let prefix = format!("state-{}", uuid::Uuid::new_v4().simple());
    let (owner, owner_token) = provision(&format!("{prefix}-owner")).await;
    let (departing, departing_token) = provision(&format!("{prefix}-departing")).await;
    let (_, outsider_token) = provision(&format!("{prefix}-outsider")).await;
    let service = Service::new(crate::routing::root());
    for departure in ["leave", "kick", "ban"] {
        let created = ok(
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .add_header("Authorization", format!("Bearer {owner_token}"), true)
                .json(&json!({"room_version":"11", "preset":"public_chat"})),
            &service,
        )
        .await;
        let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
        action(&service, &room, &departing_token, "join", json!({})).await;
        update_snapshot(&service, &room, &owner_token, &owner, "before").await;
        assert_snapshot(&service, &room, &departing_token, &owner, "before").await;
        let (actor, body) = if departure == "leave" {
            (&departing_token, json!({}))
        } else {
            (&owner_token, json!({"user_id":departing}))
        };
        action(&service, &room, actor, departure, body).await;
        update_snapshot(&service, &room, &owner_token, &owner, "after").await;
        assert_snapshot(&service, &room, &owner_token, &owner, "after").await;
        assert_snapshot(&service, &room, &departing_token, &owner, "before").await;

        // Unrelated non-members cannot read shared history. Check all GET shapes.
        for suffix in ["", "/m.room.topic", "/org.palpo.snapshot/key"] {
            let response = send(
                authed_get(
                    format!("http://localhost/_matrix/client/v3/rooms/{room}/state{suffix}"),
                    &outsider_token,
                ),
                &service,
            )
            .await;
            assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
        }
        put_state(
            &service,
            &room,
            &owner_token,
            "m.room.history_visibility",
            "",
            json!({"history_visibility":"world_readable"}),
        )
        .await;
        // Changing current visibility must not move a former member's snapshot.
        assert_snapshot(&service, &room, &departing_token, &owner, "before").await;
        assert_snapshot(&service, &room, &outsider_token, &owner, "after").await;
        if departure == "ban" {
            action(
                &service,
                &room,
                &owner_token,
                "unban",
                json!({"user_id":departing}),
            )
            .await;
        }
        action(&service, &room, &departing_token, "join", json!({})).await;
        assert_snapshot(&service, &room, &departing_token, &owner, "after").await;
        // A subsequent departure takes a fresh cap rather than the first leave.
        action(&service, &room, &departing_token, "leave", json!({})).await;
        update_snapshot(&service, &room, &owner_token, &owner, "later").await;
        assert_snapshot(&service, &room, &departing_token, &owner, "after").await;

        // Missing snapshot metadata must be an error, never a successful empty
        // state response (the old endpoint swallowed this lookup failure).
        use crate::data::schema::rooms;
        let frame = crate::room::get_frame_id(&room, None).await.unwrap();
        diesel::update(rooms::table.find(&room))
            .set(rooms::state_frame_id.eq(None::<i64>))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        let mut response = send(
            authed_get(
                format!("http://localhost/_matrix/client/v3/rooms/{room}/state"),
                &owner_token,
            ),
            &service,
        )
        .await;
        let body: Value = response.take_json().await.unwrap();
        assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND), "{body}");
        assert_eq!(body["errcode"], "M_NOT_FOUND");
        diesel::update(rooms::table.find(&room))
            .set(rooms::state_frame_id.eq(Some(frame)))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
    }
}
