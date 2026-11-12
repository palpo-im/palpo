use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

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
async fn setup() -> (Service, OwnedRoomId, OwnedUserId, OwnedUserId) {
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
    (service, room, sender, peer)
}

async fn run() {
    let (_service, room, sender, peer) = setup().await;
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

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_live_promotion_and_purge_share_fence_and_atomic_tips() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(promotion_purge());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn promotion_purge() {
    use horme::RunQueryDsl;
    let (service, room, sender, _peer) = setup().await;
    let (pdu, json) = crate::event::PduBuilder {
        event_type: "m.room.message".into(),
        timestamp: Some(UnixMillis(1)),
        content: serde_json::value::to_raw_value(
            &json!({"body":"cached remote promotion","msgtype":"m.text"}),
        )
        .unwrap(),
        ..Default::default()
    }
    .hash_sign(&sender, &room, &crate::core::RoomVersionId::V11)
    .await
    .unwrap();
    let (pdu, json, guard) = crate::event::PduBuilder::save_as_outlier(pdu, json, &sender)
        .await
        .unwrap();
    drop(guard);
    // A pending outlier is not history and must survive purge before promotion.
    data::room::timeline::purge_room_history(&room, i64::MAX)
        .await
        .unwrap();
    assert!(timeline::get_pdu(&pdu.event_id).await.is_ok());
    let event_id = pdu.event_id.clone();
    let old_tips = state::get_forward_extremities(&room).await.unwrap();
    let mut control = data::checkout().await.unwrap();
    diesel::sql_query("BEGIN")
        .execute(&mut control)
        .await
        .unwrap();
    diesel::sql_query("SELECT pg_advisory_xact_lock(1346456656,2121)")
        .execute(&mut control)
        .await
        .unwrap();
    diesel::sql_query("CREATE FUNCTION pause_forward_tip() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(1346456656,2121); RETURN NEW; END $$").execute(&mut data::db()).await.unwrap();
    diesel::sql_query("CREATE TRIGGER pause_forward_tip BEFORE INSERT ON event_forward_extremities FOR EACH ROW EXECUTE FUNCTION pause_forward_tip()").execute(&mut data::db()).await.unwrap();
    let publish_room = room.clone();
    let publish = tokio::spawn(async move {
        timeline::append_pdu(&pdu, json, &room::lock_state(&publish_room).await).await
    });
    wait_advisory_waiters(1).await;
    // DELETE and INSERT of tips are one SQL transaction; the old set is visible
    // until the new set commits, even with insertion deliberately paused.
    assert_eq!(
        state::get_forward_extremities(&room).await.unwrap(),
        old_tips
    );
    let purge_room = room.clone();
    let purge = tokio::spawn(async move {
        data::room::timeline::purge_room_history(&purge_room, i64::MAX).await
    });
    wait_advisory_waiters(2).await;
    diesel::sql_query("COMMIT")
        .execute(&mut control)
        .await
        .unwrap();
    publish.await.unwrap().unwrap();
    purge.await.unwrap().unwrap();
    assert!(timeline::get_pdu(&event_id).await.is_ok());
    assert!(
        state::get_forward_extremities(&room)
            .await
            .unwrap()
            .contains(&event_id)
    );
    // The local builder already owns the fence; its final append must not try
    // to reacquire it through another connection after the purge completes.
    let sent = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/after-purge"
        ))
        .add_header("Authorization", "Bearer alice", true)
        .json(&json!({"body":"after purge","msgtype":"m.text"})),
        &service,
    )
    .await;
    let after = timeline::get_pdu(sent["event_id"].as_str().unwrap().try_into().unwrap())
        .await
        .unwrap();
    assert!(after.prev_events.contains(&event_id));
}

async fn wait_advisory_waiters(expected: i64) {
    use horme::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Waiting {
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        count: i64,
    }
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {
            let waiting=diesel::sql_query("SELECT COUNT(*) AS count FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())").get_result::<Waiting>(&mut data::db()).await.unwrap();
            if waiting.count>=expected { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("expected publication/purge advisory waiters");
}
