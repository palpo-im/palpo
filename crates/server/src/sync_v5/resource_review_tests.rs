//! Real request/storage regressions for sync resource bounds and delivery state.
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;
use crate::PduBuilder;
use crate::core::RoomVersionId;
use crate::test_database::sql_spy::SqlSpy;

const TOKEN: &str = "sync-resource-review-token";

async fn response(builder: RequestBuilder, service: &Service) -> (StatusCode, Value) {
    let router = service.router();
    let mut response = tokio::spawn(async move { builder.send(router).await })
        .await
        .unwrap();
    let status = response.status_code.unwrap();
    (status, response.take_json().await.unwrap())
}

async fn sync(service: &Service, body: Value, pos: Option<&str>) -> (StatusCode, Value) {
    let url = format!(
        "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync?timeout=0{}",
        pos.map(|pos| format!("&pos={pos}")).unwrap_or_default()
    );
    response(
        TestClient::post(url)
            .add_header("Authorization", format!("Bearer {TOKEN}"), true)
            .json(&body),
        service,
    )
    .await
}

fn room_events<'a>(body: &'a Value, room: &RoomId, section: &str) -> &'a Vec<Value> {
    body["rooms"][room.as_str()][section]
        .as_array()
        .unwrap_or_else(|| panic!("expected room {room} {section} array in sync response: {body}"))
}

fn assert_no_room_updates(body: &Value) {
    // SyncEventsResBody intentionally omits an empty rooms map on the wire.
    if let Some(rooms) = body.get("rooms") {
        assert!(
            rooms.as_object().is_some_and(|rooms| rooms.is_empty()),
            "expected omitted or empty rooms in sync response: {body}"
        );
    }
}

fn init() {
    crate::test_database::init();
    crate::config::CONFIG.set(serde_json::from_value(json!({
        "server_name":"sync-resource.test", "db":{"url":"unused-test-config"},
        "ip_range_denylist":[], "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
    })).unwrap()).unwrap();
}

async fn append(user: &UserId, room: &RoomId, ty: &str, key: Option<String>, content: Value) {
    crate::room::timeline::build_and_append_pdu(
        PduBuilder {
            event_type: ty.into(),
            state_key: key,
            content: serde_json::value::to_raw_value(&content).unwrap(),
            ..Default::default()
        },
        user,
        room,
        &RoomVersionId::V11,
        &crate::room::lock_state(room).await,
    )
    .await
    .unwrap();
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_sync_http_bounds_and_required_state_batch_delivery() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(http_bounds());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn http_bounds() {
    init();
    let user: OwnedUserId = "@owner:sync-resource.test".try_into().unwrap();
    let device: OwnedDeviceId = "D".into();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "owner".into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&user).await.unwrap();
    data::user::device::create_device(&user, &device, TOKEN, None, None)
        .await
        .unwrap();
    let service = Service::new(crate::routing::root());
    let (status, created) = response(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", format!("Bearer {TOKEN}"), true)
            .json(&json!({"room_version":"11","preset":"public_chat"})),
        &service,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    for i in 0..120 {
        append(
            &user,
            &room,
            "m.room.message",
            None,
            json!({"msgtype":"m.text","body":format!("message {i}")}),
        )
        .await;
    }
    for i in 0..64 {
        append(
            &user,
            &room,
            "org.palpo.review.state",
            Some(format!("k{i}")),
            json!({"value":i}),
        )
        .await;
    }
    let spy = SqlSpy::install().await;
    for body in [
        json!({"conn_id":"12345678901234567"}),
        json!({"conn_id":"x".repeat(6400)}),
        json!({"conn_id":"negative","room_subscriptions":{(room.as_str()):{"timeline_limit":-1}}}),
        json!({"conn_id":"negative","room_subscriptions":{(room.as_str()):{"timeline_limit":-2}}}),
        json!({"conn_id":"negative","lists":{"all":{"timeline_limit":-1}}}),
    ] {
        spy.reset();
        let conn = body["conn_id"].as_str().unwrap().to_owned();
        let (status, error) = sync(&service, body, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
        assert!(!CONNECTIONS.lock().unwrap().contains_key(&connection_key(
            &user,
            &device,
            &Some(conn)
        )));
        assert!(
            !spy.snapshot()
                .iter()
                .any(|sql| sql.contains("sliding_sync_connections")),
            "invalid requests must not allocate connection state"
        );
    }
    for limit in [1000, i64::MAX] {
        let (listed_status, listed) = sync(&service, json!({"conn_id":"limit-list","lists":{"all":{"timeline_limit":limit,"ranges":[[0,0]]}}}), None).await;
        let (status, subscribed) = sync(&service, json!({"conn_id":"limit-sub","room_subscriptions":{(room.as_str()):{"timeline_limit":limit}}}), None).await;
        assert_eq!(
            listed_status,
            StatusCode::OK,
            "list sync limit {limit}: {listed}"
        );
        assert_eq!(status, StatusCode::OK, "{subscribed}");
        assert_eq!(room_events(&listed, &room, "timeline").len(), 100);
        assert_eq!(room_events(&subscribed, &room, "timeline").len(), 100);
    }
    let body = json!({"conn_id":"state-batch","room_subscriptions":{(room.as_str()):{
        "timeline_limit":0,"required_state":[["org.palpo.review.state","*"],["org.palpo.review.state","k0"]]
    }}});
    spy.reset();
    let (status, first) = sync(&service, body.clone(), Some("1")).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(room_events(&first, &room, "required_state").len(), 64);
    let updates = spy
        .snapshot()
        .iter()
        .filter(|sql| sql.starts_with("UPDATE sliding_sync_connections"))
        .count();
    assert!(
        (1..=5).contains(&updates),
        "64 state events must use a constant number of cache writes, got {updates}"
    );
    let (status, replay) = sync(&service, body.clone(), Some("1")).await;
    assert_eq!(status, StatusCode::OK, "replayed sync: {replay}");
    assert_eq!(
        room_events(&replay, &room, "required_state").len(),
        64,
        "a lost response must be replayable"
    );
    let (status, acknowledged) = sync(&service, body.clone(), first["pos"].as_str()).await;
    assert_eq!(status, StatusCode::OK, "acknowledged sync: {acknowledged}");
    assert_no_room_updates(&acknowledged);
    append(
        &user,
        &room,
        "org.palpo.review.state",
        Some("k0".into()),
        json!({"value":"replacement"}),
    )
    .await;
    let (status, changed) = sync(&service, body.clone(), acknowledged["pos"].as_str()).await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(room_events(&changed, &room, "required_state").len(), 1);
    let (status, settled) = sync(&service, body, changed["pos"].as_str()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "replacement acknowledgement: {settled}"
    );
    assert_no_room_updates(&settled);
    let key = connection_key(&user, &device, &Some("state-batch".into()));
    let stored = read_connection(&key).await.unwrap().unwrap();
    assert_eq!(
        stored.required_state_by_room[&room]["org.palpo.review.state"].len(),
        64,
        "replace old sn instead of accumulating history"
    );
    let (status, unsubscribed) = sync(
        &service,
        json!({"conn_id":"state-batch","unsubscribe_rooms":[room]}),
        changed["pos"].as_str(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "unsubscribe sync: {unsubscribed}");
    assert_no_room_updates(&unsubscribed);
    assert!(
        !read_connection(&key)
            .await
            .unwrap()
            .unwrap()
            .required_state_by_room
            .contains_key(&room)
    );
    spy.assert_coverage().await;
    spy.uninstall().await;
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_sync_orphans_expire_without_database_rows_and_cache_is_bounded() {
    init();
    let user: OwnedUserId = "@cache:sync-resource.test".try_into().unwrap();
    let device: OwnedDeviceId = "CACHE".into();
    let id = Some("orphan".to_owned());
    diesel::sql_query("CREATE FUNCTION review_fail_connection() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected first insert failure'; END $$")
        .execute(&mut data::db()).await.unwrap();
    diesel::sql_query("CREATE TRIGGER review_fail_connection BEFORE INSERT ON sliding_sync_connections FOR EACH ROW EXECUTE FUNCTION review_fail_connection()")
        .execute(&mut data::db()).await.unwrap();
    update_connection(&user, &device, &id, |cached| {
        cached.list_counts.insert("orphan".into(), 1);
    })
    .await;
    let key = connection_key(&user, &device, &id);
    assert!(read_connection(&key).await.unwrap().is_none());
    {
        let cache = CONNECTIONS.lock().unwrap();
        let entry = cache
            .get(&key)
            .expect("failed persistence left a memory entry");
        entry.last_access.store(
            UnixMillis::now().get() as i64 - MEMORY_CONNECTION_EXPIRY_MS - 1,
            AtomicOrdering::Relaxed,
        );
    }
    // Cleanup must remove the orphan before even trying the unavailable table.
    diesel::sql_query(
        "ALTER TABLE sliding_sync_connections RENAME TO review_connections_unavailable",
    )
    .execute(&mut data::db())
    .await
    .unwrap();
    cleanup_expired_connections().await;
    assert!(!CONNECTIONS.lock().unwrap().contains_key(&key));
    diesel::sql_query(
        "ALTER TABLE review_connections_unavailable RENAME TO sliding_sync_connections",
    )
    .execute(&mut data::db())
    .await
    .unwrap();
    diesel::sql_query("DROP TRIGGER review_fail_connection ON sliding_sync_connections")
        .execute(&mut data::db())
        .await
        .unwrap();
    for i in 0..MAX_DEVICE_CACHED_CONNECTIONS + 3 {
        drop(load_or_create_connection(&user, &device, &Some(format!("c{i}"))).await);
    }
    assert_eq!(
        CONNECTIONS
            .lock()
            .unwrap()
            .keys()
            .filter(|(u, d, _)| u == &user && d == &device)
            .count(),
        MAX_DEVICE_CACHED_CONNECTIONS
    );
    {
        let mut cache = CONNECTIONS.lock().unwrap();
        cache.clear();
        for i in 0..MAX_CACHED_CONNECTIONS {
            let user: OwnedUserId = format!("@cache{i}:sync-resource.test").try_into().unwrap();
            cache.insert(
                (user, device.clone(), None),
                Arc::new(Connection::new(SlidingSyncCache::default())),
            );
        }
    }
    drop(load_or_create_connection(&user, &device, &Some("global".into())).await);
    assert_eq!(CONNECTIONS.lock().unwrap().len(), MAX_CACHED_CONNECTIONS);
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_required_state_cas_rebases_and_failed_writes_do_not_ack_delivery() {
    init();
    let user: OwnedUserId = "@cas:sync-resource.test".try_into().unwrap();
    let device: OwnedDeviceId = "CAS".into();
    let id = Some("cas".to_owned());
    let key = connection_key(&user, &device, &id);
    update_connection(&user, &device, &id, |_| {}).await;
    let stale = CONNECTIONS.lock().unwrap().remove(&key).unwrap();
    update_connection(&user, &device, &id, |cache| {
        cache.list_counts.insert("other-instance".into(), 7);
    })
    .await;
    CONNECTIONS.lock().unwrap().insert(key.clone(), stale);
    let room: OwnedRoomId = "!cas:sync-resource.test".try_into().unwrap();
    let states: RequiredStateMap = [(
        room.clone(),
        [("org.palpo.test".into(), [("key".into(), 12)].into())].into(),
    )]
    .into();
    let mut first = SyncEventsResBody::new("20".into());
    record_required_state(&user, &device, &id, &mut first, &states, &states, &[], None)
        .await
        .unwrap();
    let stored = read_connection(&key).await.unwrap().unwrap();
    assert_eq!(stored.list_counts["other-instance"], 7);
    assert!(stored.required_state_by_room.is_empty());
    assert_eq!(stored.pending_required_state[&first.pos], states);
    acknowledge_required_state(&user, &device, &id, Some("20"))
        .await
        .unwrap();
    assert!(
        read_connection(&key)
            .await
            .unwrap()
            .unwrap()
            .required_state_by_room
            .is_empty()
    );
    acknowledge_required_state(&user, &device, &id, Some(&first.pos))
        .await
        .unwrap();
    assert_eq!(
        read_connection(&key)
            .await
            .unwrap()
            .unwrap()
            .required_state_by_room,
        states
    );
    diesel::sql_query("CREATE FUNCTION review_skip_connection_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$").execute(&mut data::db()).await.unwrap();
    diesel::sql_query("CREATE TRIGGER review_skip_connection_update BEFORE UPDATE ON sliding_sync_connections FOR EACH ROW EXECUTE FUNCTION review_skip_connection_update()")
        .execute(&mut data::db()).await.unwrap();
    let newer: RequiredStateMap = [(
        room,
        [("org.palpo.test".into(), [("key".into(), 13)].into())].into(),
    )]
    .into();
    let mut response = SyncEventsResBody::new("21".into());
    assert!(
        record_required_state(
            &user,
            &device,
            &id,
            &mut response,
            &newer,
            &newer,
            &[],
            None
        )
        .await
        .is_err()
    );
    assert_eq!(
        response.pos, "21",
        "a failed persistence must not issue a delivery token"
    );
    assert!(
        read_connection(&key)
            .await
            .unwrap()
            .unwrap()
            .pending_required_state
            .is_empty()
    );
    diesel::sql_query("DROP TRIGGER review_skip_connection_update ON sliding_sync_connections")
        .execute(&mut data::db())
        .await
        .unwrap();
    record_required_state(
        &user,
        &device,
        &id,
        &mut response,
        &newer,
        &newer,
        &[],
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        read_connection(&key)
            .await
            .unwrap()
            .unwrap()
            .pending_required_state[&response.pos],
        newer
    );
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_old_required_state_response_cannot_settle_new_configuration() {
    init();
    let user: OwnedUserId = "@config:sync-resource.test".try_into().unwrap();
    let device: OwnedDeviceId = "CONFIG".into();
    let id = Some("config-race".to_owned());
    let key = connection_key(&user, &device, &id);
    let room: OwnedRoomId = "!config:sync-resource.test".try_into().unwrap();
    let mut old_request: SyncEventsReqBody = serde_json::from_value(json!({
        "conn_id":"config-race", "room_subscriptions":{(room.as_str()):{
            "timeline_limit":0,"required_state":[["org.palpo.test","old"]]
        }}
    }))
    .unwrap();
    let (_, _, _, old_generation) = update_sync_request_with_cache_and_generation(
        user.clone(),
        device.clone(),
        &mut old_request,
    )
    .await;
    let old_config = required_state_request_config(&old_request);
    let old_states: RequiredStateMap = [(
        room.clone(),
        [("org.palpo.test".into(), [("old".into(), 12)].into())].into(),
    )]
    .into();
    // A has already constructed its old response and keeps the first instance's
    // stale cache. B then changes selectors on another instance.
    let stale_a = CONNECTIONS.lock().unwrap().remove(&key).unwrap();
    let mut new_request: SyncEventsReqBody = serde_json::from_value(json!({
        "conn_id":"config-race", "room_subscriptions":{(room.as_str()):{
            "timeline_limit":0,"required_state":[["org.palpo.test","old"],["org.palpo.test","new"]]
        }}
    }))
    .unwrap();
    let (_, _, _, new_generation) = update_sync_request_with_cache_and_generation(
        user.clone(),
        device.clone(),
        &mut new_request,
    )
    .await;
    assert!(new_generation > old_generation);
    let new_config = required_state_request_config(&new_request);
    let new_states: RequiredStateMap = [(
        room.clone(),
        [(
            "org.palpo.test".into(),
            [("old".into(), 12), ("new".into(), 13)].into(),
        )]
        .into(),
    )]
    .into();
    let mut old_response = SyncEventsResBody::new("20".into());
    record_required_state(
        &user,
        &device,
        &id,
        &mut old_response,
        &old_states,
        &RequiredStateMap::new(),
        &old_config,
        Some(old_generation),
    )
    .await
    .unwrap();
    assert!(
        read_connection(&key)
            .await
            .unwrap()
            .unwrap()
            .required_state_refresh,
        "A must not clear B's unserved refresh"
    );
    let mut new_response = SyncEventsResBody::new("20".into());
    #[cfg(feature = "unstable-msc4494")]
    {
        new_response.lists.insert(
            "main".into(),
            sync_events::v5::SyncList {
                count: 1,
                ops: vec![sync_events::v5::SyncListOp::Sync {
                    range: (0, 0),
                    room_ids: vec![room.clone()],
                }],
            },
        );
        record_list_windows_sent(&user, &device, &id, &mut new_response).await;
    }
    record_required_state(
        &user,
        &device,
        &id,
        &mut new_response,
        &new_states,
        &new_states,
        &new_config,
        Some(new_generation),
    )
    .await
    .unwrap();
    let delivered_token = new_response.pos.clone();
    #[cfg(feature = "unstable-msc4494")]
    {
        // Same position and window, but A must never borrow B's state token.
        old_response.lists = new_response.lists.clone();
        record_list_windows_sent(&user, &device, &id, &mut old_response).await;
        record_required_state(
            &user,
            &device,
            &id,
            &mut old_response,
            &old_states,
            &RequiredStateMap::new(),
            &old_config,
            Some(old_generation),
        )
        .await
        .unwrap();
        assert_ne!(old_response.pos, delivered_token);
        acknowledge_required_state(&user, &device, &id, Some(&old_response.pos))
            .await
            .unwrap();
        let stored = read_connection(&key).await.unwrap().unwrap();
        assert!(
            stored.required_state_by_room.is_empty(),
            "A's list token cannot acknowledge B's state payload"
        );
        assert_eq!(stored.pending_required_state[&delivered_token], new_states);
    }
    // A finishes after B's pending offer is durable. Force A's first CAS to
    // conflict, proving the generation guard also runs after the real rebase.
    CONNECTIONS.lock().unwrap().insert(key.clone(), stale_a);
    record_required_state(
        &user,
        &device,
        &id,
        &mut old_response,
        &old_states,
        &RequiredStateMap::new(),
        &old_config,
        Some(old_generation),
    )
    .await
    .unwrap();
    let stored = read_connection(&key).await.unwrap().unwrap();
    assert_eq!(stored.required_state_config, new_config);
    assert_eq!(
        stored.pending_required_state[&delivered_token], new_states,
        "old pruning must not discard B's new selector"
    );
    acknowledge_required_state(&user, &device, &id, Some(&delivered_token))
        .await
        .unwrap();
    assert_eq!(
        read_connection(&key)
            .await
            .unwrap()
            .unwrap()
            .required_state_by_room,
        new_states
    );
    // A -> B -> A is still a new configuration lifetime (e.g. unsubscribe and
    // re-subscribe). Identical selector fingerprints must not permit old A.
    let (_, _, _, resumed_generation) = update_sync_request_with_cache_and_generation(
        user.clone(),
        device.clone(),
        &mut old_request,
    )
    .await;
    assert!(resumed_generation > new_generation);
    assert_eq!(required_state_request_config(&old_request), old_config);
    record_required_state(
        &user,
        &device,
        &id,
        &mut old_response,
        &old_states,
        &RequiredStateMap::new(),
        &old_config,
        Some(old_generation),
    )
    .await
    .unwrap();
    let stored = read_connection(&key).await.unwrap().unwrap();
    assert!(
        stored.required_state_refresh,
        "an old equal fingerprint must not settle the resumed configuration"
    );
    assert_eq!(stored.required_state_generation, resumed_generation);
}
