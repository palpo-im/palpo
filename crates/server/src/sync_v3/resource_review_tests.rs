use super::*;
use salvo::{
    prelude::*,
    test::{RequestBuilder, ResponseExt, TestClient},
};
use serde_json::{Value, json};

async fn request(builder: RequestBuilder, service: &Service) -> Value {
    let router = service.router();
    let mut response = tokio::spawn(async move { builder.send(router).await })
        .await
        .unwrap();
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_filtered_room_skips_payloads_and_keeps_independent_membership() {
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
    config::CONFIG.set(serde_json::from_value(json!({"server_name":"filtered.example", "db":{"url":"unused"},"auto_join_rooms":[]})).unwrap()).unwrap();
    for name in ["alice", "bob"] {
        let user = UserId::parse(format!("@{name}:filtered.example")).unwrap();
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
        data::user::device::create_device(&user, &OwnedDeviceId::from("D"), name, None, None)
            .await
            .unwrap();
    }
    let service = Service::new(crate::routing::root());
    let created = request(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", "Bearer alice", true)
            .json(&json!({"room_version":"11","preset":"public_chat"})),
        &service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    request(
        TestClient::post(format!("http://localhost/_matrix/client/v3/join/{room}"))
            .add_header("Authorization", "Bearer bob", true)
            .json(&json!({})),
        &service,
    )
    .await;
    let sender = UserId::parse("@alice:filtered.example").unwrap();
    let peer = UserId::parse("@bob:filtered.example").unwrap();
    let next = BatchToken::new_publication(data::curr_sn().await.unwrap() + 1);
    let mut filter = FilterDefinition::default();
    filter.room.not_rooms.push(room.clone());
    let mut joined = HashSet::new();
    let mut left = HashSet::new();
    let spy = crate::test_database::sql_spy::SqlSpy::install().await;
    let (body, _) = load_joined_room(
        &sender,
        "D".into(),
        &room,
        false,
        None,
        None,
        next,
        false,
        &filter,
        false,
        &mut joined,
        &mut left,
    )
    .await
    .unwrap();
    assert!(body.is_empty());
    assert!(
        joined.contains(&peer),
        "excluded room must retain independent membership side effects"
    );
    assert!(
        !spy.snapshot()
            .iter()
            .any(|query| query.contains("event_receipts")),
        "filtered room fetched receipt payload"
    );
    spy.reset();
    filter.room.not_rooms.clear();
    load_joined_room(
        &sender,
        "D".into(),
        &room,
        false,
        None,
        None,
        next,
        false,
        &filter,
        false,
        &mut joined,
        &mut left,
    )
    .await
    .unwrap();
    assert!(
        spy.snapshot()
            .iter()
            .any(|query| query.contains("event_receipts")),
        "positive control did not load selected room payload"
    );
    spy.assert_coverage().await;
    spy.uninstall().await;
}
