//! Real client, admin and MAS routes with room/sync/federation-queue assertions.
use diesel::prelude::*;
use diesel_async::SimpleAsyncConnection;
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedEventId, OwnedRoomId, OwnedUserId, UnixMillis, UserId};
use crate::data::schema::*;
use crate::{config, data};

async fn send(service: &Service, token: &str, request: RequestBuilder) -> (StatusCode, Value) {
    let request = request.add_header("authorization", format!("Bearer {token}"), true);
    let router = service.router();
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

async fn ok(service: &Service, token: &str, request: RequestBuilder) -> Value {
    let (status, body) = send(service, token, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn provision(name: &str, admin: bool) -> OwnedUserId {
    let user = UserId::parse(format!("@{name}:{}", config::server_name())).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: admin,
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
    data::user::device::create_device(&user, "D".into(), name, None, None)
        .await
        .unwrap();
    user
}

async fn member(service: &Service, room: &OwnedRoomId, target: &OwnedUserId) -> Value {
    ok(
        service,
        "viewer",
        TestClient::get(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/state/m.room.member/{target}"
        )),
    )
    .await
}

async fn member_id(room: &OwnedRoomId, target: &OwnedUserId) -> OwnedEventId {
    crate::room::get_state(room, &"m.room.member".into(), target.as_str(), None)
        .await
        .unwrap()
        .event_id
        .clone()
}

async fn assert_profiles(
    service: &Service,
    rooms: &[OwnedRoomId],
    user: &OwnedUserId,
    name: Option<&str>,
    avatar: Option<&str>,
    hash: Option<&str>,
) {
    let global = data::user::get_profile(user, None).await.unwrap().unwrap();
    assert_eq!(global.display_name.as_deref(), name);
    assert_eq!(
        global
            .avatar_url
            .as_deref()
            .map(crate::core::MxcUri::as_str),
        avatar
    );
    assert_eq!(global.blurhash.as_deref(), hash);
    for room in rooms {
        let state = member(service, room, user).await;
        assert_eq!(state["displayname"].as_str(), name, "{state}");
        assert_eq!(state["avatar_url"].as_str(), avatar, "{state}");
        assert_eq!(state["xyz.amorgan.blurhash"].as_str(), hash, "{state}");
        assert_eq!(state["reason"], "keep this member field");
    }
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_profile_writes_propagate_across_routes_and_retry_partial_rooms() {
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
                "server_name":"profile-writes.example","db":{"url":"unused"},"auto_join_rooms":[],
                "admin":{"mas_secret":"profile-mas-secret"},
                "presence":{"allow_local":true,"allow_incoming":false,"allow_outgoing":false},
                "rc_message":{"per_second":10000,"burst":1000}
            }))
            .unwrap(),
        )
        .unwrap();
    let (wakeups, _receiver) = tokio::sync::mpsc::channel(4096);
    crate::sending::MPSC_SENDER.set(wakeups).unwrap();
    let target = provision("target", false).await;
    let viewer = provision("viewer", false).await;
    provision("administrator", true).await;
    let service = Service::new(crate::routing::root());
    let mut rooms = Vec::new();
    for _ in 0..2 {
        let body = ok(
            &service,
            "target",
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .json(&json!({"room_version":"11","preset":"private_chat","invite":[viewer]})),
        )
        .await;
        let room: OwnedRoomId = body["room_id"].as_str().unwrap().try_into().unwrap();
        ok(
            &service,
            "viewer",
            TestClient::post(format!(
                "http://localhost/_matrix/client/v3/rooms/{room}/join"
            ))
            .json(&json!({})),
        )
        .await;
        ok(&service,"target",TestClient::put(format!("http://localhost/_matrix/client/v3/rooms/{room}/state/m.room.member/{target}"))
            .json(&json!({"membership":"join","displayname":"initial","reason":"keep this member field"}))).await;
        // Controlled remote participant keeps the real joined-server projection
        // populated when each new membership event refreshes that projection.
        let peer: OwnedUserId = "@bridge:peer.profile.example".try_into().unwrap();
        diesel::insert_into(room_users::table)
            .values(data::room::NewDbRoomUser {
                event_id: format!("$profile-peer-{}", rooms.len()).try_into().unwrap(),
                event_sn: 1,
                room_id: room.clone(),
                room_server_id: Some(config::server_name().to_owned()),
                user_id: peer.clone(),
                user_server_id: peer.server_name().to_owned(),
                sender_id: peer,
                membership: "join".to_owned(),
                forgotten: false,
                display_name: None,
                avatar_url: None,
                state_data: None,
                created_at: UnixMillis::now(),
            })
            .execute(&mut data::db())
            .await
            .unwrap();
        let peer = crate::core::ServerName::parse("peer.profile.example").unwrap();
        assert!(data::room::add_joined_server(&room, &peer).await.unwrap());
        rooms.push(room);
    }
    rooms.sort();
    let initial = ok(
        &service,
        "viewer",
        TestClient::get("http://localhost/_matrix/client/v3/sync"),
    )
    .await;
    let client = format!("http://localhost/_matrix/client/v3/profile/{target}");
    ok(
        &service,
        "target",
        TestClient::put(format!("{client}/displayname")).json(&json!({"displayname":"Client"})),
    )
    .await;
    ok(
        &service,
        "target",
        TestClient::put(format!("{client}/avatar_url")).json(&json!({
            "avatar_url":"mxc://profiles.example/client","xyz.amorgan.blurhash":"client-hash"
        })),
    )
    .await;
    assert_profiles(
        &service,
        &rooms,
        &target,
        Some("Client"),
        Some("mxc://profiles.example/client"),
        Some("client-hash"),
    )
    .await;
    let admin = format!("http://localhost/_synapse/admin/v2/users/{target}");
    let presence_before = data::user::presence_sn(&target).await.unwrap().unwrap();
    ok(
        &service,
        "administrator",
        TestClient::put(&admin).json(&json!({
            "displayname":"Admin","avatar_url":"mxc://profiles.example/admin"
        })),
    )
    .await;
    assert_profiles(
        &service,
        &rooms,
        &target,
        Some("Admin"),
        Some("mxc://profiles.example/admin"),
        None,
    )
    .await;
    assert!(data::user::presence_sn(&target).await.unwrap().unwrap() > presence_before);
    let sync = ok(
        &service,
        "viewer",
        TestClient::get(format!(
            "http://localhost/_matrix/client/v3/sync?since={}",
            initial["next_batch"].as_str().unwrap()
        )),
    )
    .await;
    for room in &rooms {
        let events = sync["rooms"]["join"][room.as_str()]["timeline"]["events"]
            .as_array()
            .unwrap();
        assert!(
            events.iter().any(|event| event["type"] == "m.room.member"
                && event["state_key"] == target.as_str()
                && event["content"]["displayname"] == "Admin"
                && event["content"]["avatar_url"] == "mxc://profiles.example/admin"),
            "{sync}"
        );
        let id = member_id(room, &target).await;
        let queued = outgoing_requests::table
            .filter(outgoing_requests::pdu_id.eq(&id))
            .filter(outgoing_requests::server_id.eq("peer.profile.example"))
            .count()
            .get_result::<i64>(&mut data::db())
            .await
            .unwrap();
        assert_eq!(queued, 1, "updated member must enter federation queue");
    }
    // Explicit null differs from an omitted field and deletes both values.
    ok(
        &service,
        "administrator",
        TestClient::put(&admin).json(&json!({"displayname":null,"avatar_url":null})),
    )
    .await;
    assert_profiles(&service, &rooms, &target, None, None, None).await;
    ok(
        &service,
        "administrator",
        TestClient::put(&admin)
            .json(&json!({"displayname":"Admin2","avatar_url":"mxc://profiles.example/second"})),
    )
    .await;
    // Admin empty values delete rather than leave stale room names/avatars.
    ok(
        &service,
        "administrator",
        TestClient::put(&admin).json(&json!({"displayname":"","avatar_url":""})),
    )
    .await;
    assert_profiles(&service, &rooms, &target, None, None, None).await;
    ok(
        &service,
        "profile-mas-secret",
        TestClient::post("http://localhost/_synapse/mas/set_displayname")
            .json(&json!({"localpart":"target","displayname":"MAS"})),
    )
    .await;
    assert_profiles(&service, &rooms, &target, Some("MAS"), None, None).await;
    ok(
        &service,
        "profile-mas-secret",
        TestClient::post("http://localhost/_synapse/mas/unset_displayname")
            .json(&json!({"localpart":"target"})),
    )
    .await;
    assert_profiles(&service, &rooms, &target, None, None, None).await;
    ok(&service,"profile-mas-secret",TestClient::post("http://localhost/_synapse/mas/provision_user")
        .json(&json!({"localpart":"target","set_displayname":"Provisioned","set_avatar_url":"mxc://profiles.example/mas"}))).await;
    assert_profiles(
        &service,
        &rooms,
        &target,
        Some("Provisioned"),
        Some("mxc://profiles.example/mas"),
        None,
    )
    .await;
    ok(
        &service,
        "profile-mas-secret",
        TestClient::post("http://localhost/_synapse/mas/provision_user")
            .json(&json!({"localpart":"target","unset_displayname":true,"unset_avatar_url":true})),
    )
    .await;
    assert_profiles(&service, &rooms, &target, None, None, None).await;
    // A failure in the second room must be visible to the HTTP caller. The
    // global profile and first room are committed; an identical retry repairs
    // the other room without publishing the first member event twice.
    let escaped = rooms[1].as_str().replace('\'', "''");
    let fault = format!(
        "CREATE FUNCTION fail_profile_member() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.room_id = '{escaped}' AND NEW.json_data->>'type' = 'm.room.member' AND NEW.json_data->'content'->>'displayname' = 'Retry' THEN RAISE EXCEPTION 'injected member append failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER fail_profile_member BEFORE INSERT OR UPDATE ON event_datas FOR EACH ROW EXECUTE FUNCTION fail_profile_member();"
    );
    data::checkout()
        .await
        .unwrap()
        .batch_execute(&fault)
        .await
        .unwrap();
    let (status, body) = send(
        &service,
        "administrator",
        TestClient::put(&admin).json(&json!({"displayname":"Retry"})),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(
        data::user::display_name(&target).await.unwrap().as_deref(),
        Some("Retry")
    );
    assert_eq!(
        member(&service, &rooms[0], &target).await["displayname"],
        "Retry"
    );
    assert!(member(&service, &rooms[1], &target).await["displayname"].is_null());
    let published = member_id(&rooms[0], &target).await;
    data::checkout()
        .await
        .unwrap()
        .batch_execute(
            "DROP TRIGGER fail_profile_member ON event_datas; DROP FUNCTION fail_profile_member();",
        )
        .await
        .unwrap();
    ok(
        &service,
        "administrator",
        TestClient::put(&admin).json(&json!({"displayname":"Retry"})),
    )
    .await;
    assert_profiles(&service, &rooms, &target, Some("Retry"), None, None).await;
    assert_eq!(
        member_id(&rooms[0], &target).await,
        published,
        "retry duplicated the successful room update"
    );
    // A later failure can happen after the member is already current. A same-
    // value retry must requeue that exact PDU rather than skip the room forever.
    let escaped = rooms[0].as_str().replace('\'', "''");
    let fault = format!(
        "CREATE FUNCTION fail_profile_queue() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.kind = 'normal' AND NEW.server_id = 'peer.profile.example' AND EXISTS (SELECT 1 FROM event_datas WHERE event_id = NEW.pdu_id AND room_id = '{escaped}' AND json_data->>'type' = 'm.room.member' AND json_data->'content'->>'displayname' = 'QueueRetry') THEN RAISE EXCEPTION 'injected profile federation queue failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER fail_profile_queue BEFORE INSERT ON outgoing_requests FOR EACH ROW EXECUTE FUNCTION fail_profile_queue();"
    );
    data::checkout()
        .await
        .unwrap()
        .batch_execute(&fault)
        .await
        .unwrap();
    let (status, body) = send(
        &service,
        "administrator",
        TestClient::put(&admin).json(&json!({"displayname":"QueueRetry"})),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let awaiting_queue = member_id(&rooms[0], &target).await;
    assert_ne!(awaiting_queue, published);
    assert_eq!(
        member(&service, &rooms[0], &target).await["displayname"],
        "QueueRetry",
        "the enqueue failure must occur after member publication"
    );
    assert_eq!(
        member(&service, &rooms[1], &target).await["displayname"],
        "Retry"
    );
    let queued_before: i64 = outgoing_requests::table
        .filter(outgoing_requests::pdu_id.eq(&awaiting_queue))
        .filter(outgoing_requests::server_id.eq("peer.profile.example"))
        .count()
        .get_result(&mut data::db())
        .await
        .unwrap();
    assert_eq!(queued_before, 0);
    data::checkout().await.unwrap().batch_execute("DROP TRIGGER fail_profile_queue ON outgoing_requests; DROP FUNCTION fail_profile_queue();").await.unwrap();
    ok(
        &service,
        "administrator",
        TestClient::put(&admin).json(&json!({"displayname":"QueueRetry"})),
    )
    .await;
    assert_profiles(&service, &rooms, &target, Some("QueueRetry"), None, None).await;
    assert_eq!(
        member_id(&rooms[0], &target).await,
        awaiting_queue,
        "enqueue retry created a new member event"
    );
    let queued_after: i64 = outgoing_requests::table
        .filter(outgoing_requests::pdu_id.eq(&awaiting_queue))
        .filter(outgoing_requests::server_id.eq("peer.profile.example"))
        .count()
        .get_result(&mut data::db())
        .await
        .unwrap();
    assert_eq!(
        queued_after, 1,
        "same-value retry did not restore federation delivery"
    );
    // Client deletion shares the same field-specific propagation path.
    ok(
        &service,
        "target",
        TestClient::delete(format!("{client}/displayname")),
    )
    .await;
    ok(&service,"target",TestClient::put(format!("{client}/avatar_url"))
        .json(&json!({"avatar_url":"mxc://profiles.example/remove","xyz.amorgan.blurhash":"old-hash"}))).await;
    ok(
        &service,
        "target",
        TestClient::delete(format!("{client}/avatar_url")),
    )
    .await;
    assert_profiles(&service, &rooms, &target, None, None, None).await;
}
