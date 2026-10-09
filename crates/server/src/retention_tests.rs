//! Run each HTTP retention test alone with an empty dedicated test database.
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::json;

use super::*;
use crate::core::{OwnedDeviceId, UserId};
use crate::data::user::{NewDbAccessToken, NewDbUser, NewDbUserDevice};

async fn provision() {
    let id = UserId::parse("@alice:retention.example").unwrap();
    let mut conn = connect().await.unwrap();
    diesel::insert_into(users::table)
        .values(NewDbUser {
            id: id.clone(),
            ty: None,
            is_admin: true,
            is_guest: false,
            is_local: true,
            localpart: "alice".to_owned(),
            server_name: id.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .execute(&mut conn)
        .await
        .unwrap();
    diesel::insert_into(user_devices::table)
        .values(NewDbUserDevice {
            user_id: id.clone(),
            device_id: "D".into(),
            display_name: None,
            user_agent: None,
            is_hidden: false,
            last_seen_ip: None,
            last_seen_at: None,
            created_at: UnixMillis::now(),
        })
        .execute(&mut conn)
        .await
        .unwrap();
    diesel::insert_into(user_access_tokens::table)
        .values(NewDbAccessToken::new(
            id,
            OwnedDeviceId::from("D"),
            "retention-token".to_owned(),
            None,
        ))
        .execute(&mut conn)
        .await
        .unwrap();
}

async fn get(service: &Service, path: &str) -> Response {
    send(
        TestClient::get(format!("http://localhost{path}")).add_header(
            "Authorization",
            "Bearer retention-token",
            true,
        ),
        service,
    )
    .await
}

async fn put(service: &Service, path: &str, body: Value) -> Response {
    send(
        TestClient::put(format!("http://localhost{path}"))
            .add_header("Authorization", "Bearer retention-token", true)
            .json(&body),
        service,
    )
    .await
}

async fn send(request: RequestBuilder, service: &Service) -> Response {
    // Dispatch in its own task, as the HTTP server does. The unoptimized room
    // creation future must not share the long fixture's polling stack on Windows.
    let router = service.router();
    tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap()
}

async fn body(mut response: Response) -> Value {
    let json = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{json}");
    json
}

async fn setup(enable: bool) -> Service {
    crate::test_database::init();
    config::CONFIG.set(serde_json::from_value(json!({
        "server_name":"retention.example", "db":{"url":"unused-test-config"}, "ip_range_denylist":[],
        "retention":{"enable":enable, "policies":{"*":{"max_lifetime":1000}}}
    })).unwrap()).unwrap();
    provision().await;
    Service::new(crate::routing::root())
}

fn run_http_test<F: std::future::Future<Output = ()> + 'static>(test: fn() -> F) {
    // Existing unoptimized Windows room creation overflows the harness's 2 MiB
    // stack even with all retention read checks removed. Isolate the fixture,
    // including its runtime, rather than depending on a process environment flag.
    std::thread::Builder::new()
        .name("retention-http".to_owned())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(test());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn retention_http_expiry_policy_changes_and_restart_recovery() {
    run_http_test(expiry_policy_changes_and_restart_recovery);
}

async fn expiry_policy_changes_and_restart_recovery() {
    let service = setup(true).await;
    let mut created = send(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", "Bearer retention-token", true)
            .json(&json!({"room_version":"11", "preset":"public_chat"})),
        &service,
    )
    .await;
    let created_body = created.take_json::<Value>().await.unwrap();
    assert_eq!(created.status_code, Some(StatusCode::OK), "{created_body}");
    let room_id = RoomId::parse(created_body["room_id"].as_str().unwrap()).unwrap();
    let prefix = format!("/_matrix/client/v3/rooms/{room_id}");
    let config_body = body(
        get(
            &service,
            "/_matrix/client/unstable/org.matrix.msc1763/retention/configuration",
        )
        .await,
    )
    .await;
    assert_eq!(
        config_body,
        json!({"policies":{"*":{"max_lifetime":1000}},"limits":{}})
    );
    assert_eq!(
        effective_policy(&room_id).await.unwrap().max_lifetime,
        Some(1000)
    );
    let versions = body(get(&service, "/_matrix/client/versions").await).await;
    assert_eq!(versions["unstable_features"]["org.matrix.msc1763"], true);

    for policy in [
        json!({"min_lifetime":2,"max_lifetime":1}),
        json!({"max_lifetime":-1}),
    ] {
        let response = put(&service, &format!("{prefix}/state/{EVENT_TYPE}"), policy).await;
        assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    }
    assert_eq!(
        put(
            &service,
            &format!("{prefix}/state/{EVENT_TYPE}/nonempty"),
            json!({})
        )
        .await
        .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
    let old = body(
        put(
            &service,
            &format!("{prefix}/send/m.room.message/old"),
            json!({"msgtype":"m.text","body":"retentionneedle private payload"}),
        )
        .await,
    )
    .await;
    let old_id = crate::core::EventId::parse(old["event_id"].as_str().unwrap()).unwrap();
    diesel::sql_query("INSERT INTO delayed_events (delay_id, user_id, room_id, event_type, content, delay_ms, txn_id, running_since, send_at, created_at, event_id, finalized_at) SELECT 'retention-delayed', json_data->>'sender', room_id, 'm.room.message', (json_data->'content')::jsonb, 1000, 'retention-delayed', 0, 0, 0, event_id, 0 FROM event_datas WHERE event_id = $1")
        .bind::<diesel::sql_types::Text,_>(old_id.as_str()).execute(&mut connect().await.unwrap()).await.unwrap();
    let terminal = body(
        put(
            &service,
            &format!("{prefix}/send/m.room.message/terminal"),
            json!({"msgtype":"m.text","body":"terminal payload"}),
        )
        .await,
    )
    .await;
    let terminal_id = crate::core::EventId::parse(terminal["event_id"].as_str().unwrap()).unwrap();
    // Make both message timestamps expired without a timing-sensitive sleep.
    for event_id in [&old_id, &terminal_id] {
        diesel::update(events::table.find(event_id))
            .set(events::origin_server_ts.eq(UnixMillis(0)))
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();
        diesel::sql_query("UPDATE event_datas SET json_data = jsonb_set(json_data::jsonb, '{origin_server_ts}', '0')::json WHERE event_id = $1")
            .bind::<diesel::sql_types::Text,_>(event_id.as_str()).execute(&mut connect().await.unwrap()).await.unwrap();
    }
    // Current DAG tips retain their payload; older messages expire on first read.
    assert_eq!(
        body(get(&service, &format!("{prefix}/event/{terminal_id}")).await).await["content"]["body"],
        "terminal payload"
    );
    assert_eq!(
        get(&service, &format!("{prefix}/event/{old_id}"))
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    assert!(was_expired(&old_id).await.unwrap());
    assert_eq!(
        delayed_events::table
            .filter(delayed_events::event_id.eq(&old_id))
            .select(delayed_events::content)
            .first::<Value>(&mut connect().await.unwrap())
            .await
            .unwrap(),
        json!({})
    );
    let (_, federation_json) = room::timeline::get_pdu_and_data(&old_id).await.unwrap();
    assert_eq!(
        serde_json::to_value(federation_json).unwrap()["content"],
        json!({})
    );
    let pdu_json = room::timeline::get_pdu_json(&old_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !serde_json::to_string(&pdu_json)
            .unwrap()
            .contains("retentionneedle")
    );
    assert_eq!(
        get(&service, &format!("{prefix}/context/{old_id}"))
            .await
            .status_code,
        Some(StatusCode::FORBIDDEN)
    );
    let messages = body(get(&service, &format!("{prefix}/messages?dir=b&limit=100")).await).await;
    assert!(!messages.to_string().contains("retentionneedle"));
    let sync = body(get(&service, "/_matrix/client/v3/sync?timeout=0").await).await;
    assert!(!sync.to_string().contains("retentionneedle"));
    let sliding_sync = body(
        send(
            TestClient::post(
                "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync",
            )
            .add_header("Authorization", "Bearer retention-token", true)
            .json(&json!({"lists":{"all":{"ranges":[[0,10]],"timeline_limit":100}}})),
            &service,
        )
        .await,
    )
    .await;
    assert!(!sliding_sync.to_string().contains("retentionneedle"));
    let search = body(
        send(
            TestClient::post("http://localhost/_matrix/client/v3/search")
                .add_header("Authorization", "Bearer retention-token", true)
                .json(
                    &json!({"search_categories":{"room_events":{"search_term":"retentionneedle"}}}),
                ),
            &service,
        )
        .await,
    )
    .await;
    assert_eq!(
        search["search_categories"]["room_events"]["results"],
        json!([])
    );

    // A longer room policy takes precedence over the default; it cannot undo
    // content expiry already committed. Backfill cannot restore the payload either.
    body(
        put(
            &service,
            &format!("{prefix}/state/{EVENT_TYPE}"),
            json!({"max_lifetime":crate::core::events::room::retention::MAX_LIFETIME}),
        )
        .await,
    )
    .await;
    assert_eq!(
        effective_policy(&room_id).await.unwrap().max_lifetime,
        Some(crate::core::events::room::retention::MAX_LIFETIME)
    );
    diesel::sql_query("UPDATE event_datas SET json_data = jsonb_set(json_data::jsonb, '{content}', '{\"body\":\"resurrectedsecret\",\"msgtype\":\"m.text\"}')::json WHERE event_id = $1")
        .bind::<diesel::sql_types::Text,_>(old_id.as_str()).execute(&mut connect().await.unwrap()).await.unwrap();
    assert_eq!(
        get(&service, &format!("{prefix}/event/{old_id}"))
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    assert!(
        !serde_json::to_string(&room::timeline::get_pdu_json(&old_id).await.unwrap())
            .unwrap()
            .contains("resurrectedsecret")
    );
    // A late indexer must not recreate search entries after expiry either.
    diesel::update(delayed_events::table.filter(delayed_events::event_id.eq(&old_id)))
        .set(delayed_events::content.eq(json!({"body":"resurrectedsecret"})))
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        delayed_events::table
            .filter(delayed_events::event_id.eq(&old_id))
            .select(delayed_events::content)
            .first::<Value>(&mut connect().await.unwrap())
            .await
            .unwrap(),
        json!({})
    );
    diesel::sql_query("INSERT INTO event_searches (event_id, event_sn, room_id, sender_id, key, vector, origin_server_ts) SELECT id, sn, room_id, sender_id, 'content.message', to_tsvector('english', 'resurrectedsecret'), origin_server_ts FROM events WHERE id = $1")
        .bind::<diesel::sql_types::Text,_>(old_id.as_str()).execute(&mut connect().await.unwrap()).await.unwrap();
    assert_eq!(
        event_searches::table
            .filter(event_searches::event_id.eq(&old_id))
            .count()
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap(),
        0
    );
    // A short policy change expires the former tip. Sweep reconstructs all work
    // from persisted room state/events, as it does on a fresh server startup.
    body(
        put(
            &service,
            &format!("{prefix}/state/{EVENT_TYPE}"),
            json!({"max_lifetime":0}),
        )
        .await,
    )
    .await;
    sweep().await.unwrap();
    sweep().await.unwrap();
    assert!(was_expired(&terminal_id).await.unwrap());
    let restart = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("retention_restart_probe")
        .arg("--ignored")
        .env("PALPO_RETENTION_RESTART_PROBE", "1")
        .env("PALPO_RETENTION_OLD_EVENT", old_id.as_str())
        .env("PALPO_RETENTION_ROOM", room_id.as_str())
        .output()
        .unwrap();
    assert!(
        restart.status.success(),
        "restart probe failed: {} {}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    let state = body(get(&service, &format!("{prefix}/state")).await).await;
    assert!(state.to_string().contains("m.room.create"));
    body(
        put(
            &service,
            &format!("{prefix}/send/m.room.message/after-expiry"),
            json!({"msgtype":"m.text","body":"room remains usable"}),
        )
        .await,
    )
    .await;
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn retention_http_disabled_is_not_advertised() {
    run_http_test(disabled_is_not_advertised);
}

async fn disabled_is_not_advertised() {
    let service = setup(false).await;
    assert_eq!(
        get(
            &service,
            "/_matrix/client/unstable/org.matrix.msc1763/retention/configuration"
        )
        .await
        .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    let versions = body(get(&service, "/_matrix/client/versions").await).await;
    assert!(
        versions["unstable_features"]
            .get("org.matrix.msc1763")
            .is_none()
    );
    assert!(validate_local(EVENT_TYPE, Some(""), "{}").is_err());
    sweep().await.unwrap();
    body(
        send(
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .add_header("Authorization", "Bearer retention-token", true)
                .json(&json!({"room_version":"11", "preset":"public_chat"})),
            &service,
        )
        .await,
    )
    .await;
}

#[test]
#[ignore = "internal subprocess probe; launched by retention_http_expiry"]
fn retention_restart_probe() {
    run_http_test(restart_probe);
}

async fn restart_probe() {
    if std::env::var("PALPO_RETENTION_RESTART_PROBE").as_deref() != Ok("1") {
        return;
    }
    let url = std::env::var("PALPO_TEST_DATABASE_URL").unwrap();
    let db =
        serde_json::from_value(json!({"url":url,"pool_size":10,"statement_timeout":5000})).unwrap();
    crate::data::init(&db).unwrap();
    config::CONFIG.set(serde_json::from_value(json!({
        "server_name":"retention.example", "db":{"url":"unused-test-config"}, "ip_range_denylist":[],
        "retention":{"enable":true,"policies":{"*":{"max_lifetime":1000}}}
    })).unwrap()).unwrap();
    sweep().await.unwrap();
    let old_id =
        crate::core::EventId::parse(std::env::var("PALPO_RETENTION_OLD_EVENT").unwrap()).unwrap();
    assert!(was_expired(&old_id).await.unwrap());
    let service = Service::new(crate::routing::root());
    let room = std::env::var("PALPO_RETENTION_ROOM").unwrap();
    assert_eq!(
        get(
            &service,
            &format!("/_matrix/client/v3/rooms/{room}/event/{old_id}")
        )
        .await
        .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    let state = body(get(&service, &format!("/_matrix/client/v3/rooms/{room}/state")).await).await;
    assert!(state.to_string().contains("m.room.create"));
}
