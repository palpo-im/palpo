use std::time::Duration;

use diesel::prelude::*;
use diesel_async::SimpleAsyncConnection;
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{RoomId, UnixMillis, UserId};
use crate::data::schema::{e2e_room_keys, e2e_room_keys_versions};
use crate::data::user::key_backup::{self, NewDbRoomKeysVersion};

const TOKEN: &str = "backup-mutation-token";
const PREFIX: &str = "http://localhost/_matrix/client/v3/room_keys";

async fn request(service: &Service, request: RequestBuilder) -> (StatusCode, Value) {
    let router = service.router();
    let request = request.add_header("authorization", format!("Bearer {TOKEN}"), true);
    let mut response = tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap();
    let body = response.take_json::<Value>().await.unwrap();
    (response.status_code.unwrap(), body)
}

async fn ok(service: &Service, builder: RequestBuilder) -> Value {
    let (status, body) = request(service, builder).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn key(verified: bool, index: u64, forwarded: u64, marker: &str) -> Value {
    json!({
        "is_verified":verified, "first_message_index":index, "forwarded_count":forwarded,
        "session_data":{"ciphertext":marker,"ephemeral":"ZXBoZW1lcmFs","mac":"bWFj"}
    })
}

fn etag(value: &Value) -> i64 {
    value["etag"].as_str().unwrap().parse().unwrap()
}

fn assert_mutation(value: &Value, count: u64) {
    assert_eq!(value["count"], count, "{value}");
    assert_eq!(value.as_object().unwrap().len(), 2, "{value}");
    let _ = etag(value);
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_key_backup_mutations_are_atomic_and_protocol_compatible() {
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
    crate::config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"backup-mutations.example", "db":{"url":"unused"}
            }))
            .unwrap(),
        )
        .unwrap();
    let user = UserId::parse("@alice:backup-mutations.example").unwrap();
    crate::data::user::create_user(&crate::data::user::NewDbUser {
        id: user.clone(),
        localpart: user.localpart().into(),
        server_name: user.server_name().to_owned(),
        is_local: true,
        ty: None,
        is_admin: false,
        is_guest: false,
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    crate::data::user::device::create_device(&user, "BACKUP".into(), TOKEN, None, None)
        .await
        .unwrap();
    let service = Service::new(crate::routing::root());
    let algorithm = json!({
        "algorithm":"m.megolm_backup.v1.curve25519-aes-sha2",
        "auth_data":{"public_key":"Zml4dHVyZQ","signatures":{}}
    });
    let created = ok(
        &service,
        TestClient::post(format!("{PREFIX}/version")).json(&algorithm),
    )
    .await;
    let version: i64 = created["version"].as_str().unwrap().parse().unwrap();
    let version_url = format!("{PREFIX}/version/{version}");
    let initial = ok(&service, TestClient::get(&version_url)).await;
    assert_eq!(initial["version"], created["version"]);

    // Use the complete production router: registering this as POST must fail.
    let mut signed = algorithm.clone();
    signed["auth_data"]["signatures"] = json!({user.as_str():{"ed25519:BACKUP":"c2lnbmF0dXJl"}});
    assert_eq!(
        ok(&service, TestClient::put(&version_url).json(&signed)).await,
        json!({})
    );
    let updated = ok(&service, TestClient::get(&version_url)).await;
    assert_eq!(updated["auth_data"], signed["auth_data"]);
    assert_eq!(updated["version"], initial["version"]);
    assert!(etag(&updated) > etag(&initial));

    let room = RoomId::parse("!backup:example.org").unwrap();
    let session_url = format!("{PREFIX}/keys/{room}/first?version={version}");
    let first = key(false, 10, 5, "Zmlyc3Q");
    let stored = ok(&service, TestClient::put(&session_url).json(&first)).await;
    assert_mutation(&stored, 1);
    assert_eq!(etag(&stored), etag(&updated) + 1);
    assert_eq!(ok(&service, TestClient::get(&session_url)).await, first);
    let better = key(true, 20, 9, "YmV0dGVy");
    let replaced = ok(&service, TestClient::put(&session_url).json(&better)).await;
    assert_mutation(&replaced, 1);
    assert_eq!(etag(&replaced), etag(&stored) + 1);
    let worse = key(false, 0, 0, "d29yc2U");
    let ignored = ok(&service, TestClient::put(&session_url).json(&worse)).await;
    assert_eq!(ignored, replaced, "rejected downgrade changed the etag");
    assert_eq!(ok(&service, TestClient::get(&session_url)).await, better);

    // Every comparison tier is checked independently, including ties with
    // different ciphertext: only strictly better metadata may replace a key.
    for (number, (old, candidate, replace)) in [
        (key(true, 10, 3, "b2xk"), key(false, 0, 0, "bmV3"), false),
        (key(false, 0, 0, "b2xk"), key(true, 10, 9, "bmV3"), true),
        (key(true, 10, 3, "b2xk"), key(true, 9, 99, "bmV3"), true),
        (key(true, 10, 3, "b2xk"), key(true, 11, 0, "bmV3"), false),
        (key(false, 10, 3, "b2xk"), key(false, 10, 2, "bmV3"), true),
        (key(false, 10, 3, "b2xk"), key(false, 10, 4, "bmV3"), false),
        (key(true, 10, 3, "b2xk"), key(true, 10, 3, "bmV3"), false),
        (key(false, 10, 3, "b2xk"), key(false, 9, 99, "bmV3"), true),
    ]
    .into_iter()
    .enumerate()
    {
        let url = format!("{PREFIX}/keys/{room}/quality-{number}?version={version}");
        let before = ok(&service, TestClient::put(&url).json(&old)).await;
        let after = ok(&service, TestClient::put(&url).json(&candidate)).await;
        assert_eq!(etag(&after), etag(&before) + i64::from(replace));
        assert_eq!(
            ok(&service, TestClient::get(&url)).await,
            if replace { candidate } else { old },
            "quality case {number}"
        );
    }

    concurrent_uploads(&service, &user, version, &room).await;
    rollback_content_and_etag(&service, &user, version, &room).await;

    let before_delete = ok(&service, TestClient::get(&version_url)).await;
    let deleted = ok(&service, TestClient::delete(&session_url)).await;
    assert_eq!(etag(&deleted), etag(&before_delete) + 1);
    let no_change = ok(&service, TestClient::delete(&session_url)).await;
    assert_eq!(deleted, no_change, "empty deletion changed the etag");
    let room_url = format!("{PREFIX}/keys/{room}?version={version}");
    let room_deleted = ok(&service, TestClient::delete(&room_url)).await;
    assert_mutation(&room_deleted, 0);
    assert_eq!(etag(&room_deleted), etag(&deleted) + 1);

    // Exercise the room and global upload/delete variants too. A multi-key
    // upload is a single content mutation with one coherent response snapshot.
    let room_keys = json!({"sessions":{"one":first,"two":better}});
    let room_added = ok(&service, TestClient::put(&room_url).json(&room_keys)).await;
    assert_mutation(&room_added, 2);
    assert_eq!(etag(&room_added), etag(&room_deleted) + 1);
    let all_url = format!("{PREFIX}/keys?version={version}");
    let bulk = json!({"rooms":{"!another:example.org":{"sessions":{"three":first}}}});
    let all_added = ok(&service, TestClient::put(&all_url).json(&bulk)).await;
    assert_mutation(&all_added, 3);
    assert_eq!(etag(&all_added), etag(&room_added) + 1);
    let all_deleted = ok(&service, TestClient::delete(&all_url)).await;
    assert_mutation(&all_deleted, 0);
    assert_eq!(etag(&all_deleted), etag(&all_added) + 1);
    let final_version = ok(&service, TestClient::get(&version_url)).await;
    assert_eq!(final_version["etag"], all_deleted["etag"]);
    assert_eq!(final_version["count"], 0);

    version_creation_fences_uploads(&service, &user, version, &room, &algorithm).await;
    let latest = ok(&service, TestClient::get(format!("{PREFIX}/version"))).await;
    let newest: i64 = latest["version"].as_str().unwrap().parse().unwrap();
    assert!(newest > version);
    ok(
        &service,
        TestClient::delete(format!("{PREFIX}/version/{newest}")),
    )
    .await;
    let retired_etag = key_backup::get_etag(&user, newest).await.unwrap();
    ok(
        &service,
        TestClient::delete(format!("{PREFIX}/version/{newest}")),
    )
    .await;
    assert_eq!(
        key_backup::get_etag(&user, newest).await.unwrap(),
        retired_etag
    );
    let retired_url = format!("{PREFIX}/keys/{room}/retired?version={newest}");
    let (status, body) = request(&service, TestClient::put(&retired_url).json(&first)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["errcode"], "M_WRONG_ROOM_KEYS_VERSION");
    assert_eq!(body["current_version"], version.to_string());
    assert!(
        key_backup::get_key_for_session(&user, newest, &room, &"retired".into())
            .await
            .unwrap()
            .is_none()
    );
    ok(&service, TestClient::delete(&version_url)).await;
    let (status, body) = request(&service, TestClient::delete(&all_url)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

async fn concurrent_uploads(service: &Service, user: &UserId, version: i64, room: &RoomId) {
    let mut gate = crate::data::checkout().await.unwrap();
    gate.batch_execute(
        "CREATE FUNCTION backup_review_delay_worse() RETURNS trigger LANGUAGE plpgsql AS $$ \
        BEGIN IF NEW.session_id = 'race-session' AND NOT NEW.is_verified THEN \
        PERFORM pg_advisory_xact_lock(7141322); END IF; RETURN NEW; END $$; \
        CREATE TRIGGER backup_review_delay_worse BEFORE INSERT ON e2e_room_keys \
        FOR EACH ROW EXECUTE FUNCTION backup_review_delay_worse();",
    )
    .await
    .unwrap();
    // Commit trigger DDL before opening the gate transaction. Combining both
    // in a single simple-query batch retains CREATE TRIGGER's table lock and
    // blocks INSERT before it can ever reach the advisory-lock gate.
    gate.batch_execute("BEGIN; SELECT pg_advisory_xact_lock(7141322);")
        .await
        .unwrap();
    let url = format!("{PREFIX}/keys/{room}/race-session?version={version}");
    let worse = key(false, 0, 0, "d29yc2U");
    let better = key(true, 50, 20, "YmV0dGVy");
    let mut worse_request = Box::pin(request(service, TestClient::put(&url).json(&worse)));
    tokio::select! {
        result = &mut worse_request => panic!("worse request passed the closed insert gate: {result:?}"),
        result = tokio::time::timeout(Duration::from_secs(3), wait_for_advisory_waiter(true)) => result.unwrap(),
    }
    // The worse insert has completed any pre-read and is now held in its
    // trigger. A stale read-then-unconditional-upsert allows better to finish
    // first, after which worse would overwrite it when this gate opens.
    let mut better_request = Box::pin(request(service, TestClient::put(&url).json(&better)));
    let early_better = tokio::select! {
        result = &mut better_request => Some(result),
        result = tokio::time::timeout(Duration::from_secs(3), wait_for_advisory_waiter(false)) => {
            result.unwrap();
            None
        },
    };
    gate.batch_execute("COMMIT").await.unwrap();
    let (worse_status, worse_body) = worse_request.await;
    assert_eq!(worse_status, StatusCode::OK, "{worse_body}");
    let (better_status, better_body) = if let Some(result) = early_better {
        result
    } else {
        better_request.await
    };
    assert_eq!(better_status, StatusCode::OK, "{better_body}");
    gate.batch_execute(
        "DROP TRIGGER backup_review_delay_worse ON e2e_room_keys; \
        DROP FUNCTION backup_review_delay_worse();",
    )
    .await
    .unwrap();
    assert_eq!(ok(service, TestClient::get(&url)).await, better);
    let stored = key_backup::get_key_for_session(user, version, room, &"race-session".into())
        .await
        .unwrap()
        .unwrap();
    assert!(stored.is_verified);
    assert_eq!(stored.first_message_index, Some(50));
}

async fn wait_for_advisory_waiter(insert_gate: bool) {
    loop {
        let waiting = diesel::select(diesel::dsl::sql::<diesel::sql_types::Bool>(
            if insert_gate {
                "EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' \
                AND objid = 7141322 AND NOT granted)"
            } else {
                "EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' \
                AND objid <> 7141322 AND NOT granted)"
            },
        ))
        .get_result::<bool>(&mut crate::data::db())
        .await
        .unwrap();
        if waiting {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn rollback_content_and_etag(service: &Service, user: &UserId, version: i64, room: &RoomId) {
    let before = key_backup::get_etag(user, version).await.unwrap();
    let mut conn = crate::data::checkout().await.unwrap();
    conn.batch_execute(
        "CREATE FUNCTION backup_review_fail_etag() RETURNS trigger LANGUAGE plpgsql AS $$ \
        BEGIN IF NEW.etag <> OLD.etag THEN RAISE EXCEPTION 'injected etag failure'; \
        END IF; RETURN NEW; END $$; CREATE TRIGGER backup_review_fail_etag \
        BEFORE UPDATE ON e2e_room_keys_versions FOR EACH ROW \
        EXECUTE FUNCTION backup_review_fail_etag();",
    )
    .await
    .unwrap();
    let url = format!("{PREFIX}/keys/{room}/rollback?version={version}");
    let (status, body) = request(
        service,
        TestClient::put(&url).json(&key(true, 0, 0, "cm9sbGJhY2s")),
    )
    .await;
    let race_url = format!("{PREFIX}/keys/{room}/race-session?version={version}");
    let (delete_status, delete_body) = request(service, TestClient::delete(&race_url)).await;
    conn.batch_execute(
        "DROP TRIGGER backup_review_fail_etag ON e2e_room_keys_versions; \
        DROP FUNCTION backup_review_fail_etag();",
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(
        delete_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{delete_body}"
    );
    assert_eq!(key_backup::get_etag(user, version).await.unwrap(), before);
    assert!(
        key_backup::get_key_for_session(user, version, room, &"rollback".into())
            .await
            .unwrap()
            .is_none(),
        "key content committed without its etag"
    );
    assert!(
        key_backup::get_key_for_session(user, version, room, &"race-session".into())
            .await
            .unwrap()
            .is_some(),
        "key deletion committed without its etag"
    );
}

async fn version_creation_fences_uploads(
    service: &Service,
    user: &UserId,
    version: i64,
    room: &RoomId,
    algorithm: &Value,
) {
    let before = key_backup::get_etag(user, version).await.unwrap();
    let mut creator = crate::data::checkout().await.unwrap();
    creator.batch_execute("BEGIN").await.unwrap();
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 7141321))")
        .bind::<diesel::sql_types::Text, _>(user.as_str())
        .execute(&mut *creator)
        .await
        .unwrap();
    let newest = version + 1;
    diesel::insert_into(e2e_room_keys_versions::table)
        .values(NewDbRoomKeysVersion {
            user_id: user.to_owned(),
            version: newest,
            algorithm: algorithm.clone(),
            auth_data: json!({}),
            created_at: UnixMillis::now(),
        })
        .execute(&mut *creator)
        .await
        .unwrap();
    let url = format!("{PREFIX}/keys/{room}/stale-version?version={version}");
    let mut pending = Box::pin(request(
        service,
        TestClient::put(&url).json(&key(true, 0, 0, "c3RhbGU")),
    ));
    let early = tokio::select! {
        result = &mut pending => Some(result),
        result = tokio::time::timeout(Duration::from_secs(3), wait_for_advisory_waiter(false)) => {
            result.unwrap();
            None
        },
    };
    creator.batch_execute("COMMIT").await.unwrap();
    let (status, body) = if let Some(result) = early {
        result
    } else {
        pending.await
    };
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["errcode"], "M_WRONG_ROOM_KEYS_VERSION");
    assert_eq!(body["current_version"], newest.to_string());
    assert_eq!(key_backup::get_etag(user, version).await.unwrap(), before);
    let stale_count = e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(user))
        .filter(e2e_room_keys::session_id.eq("stale-version"))
        .count()
        .get_result::<i64>(&mut crate::data::db())
        .await
        .unwrap();
    assert_eq!(stale_count, 0);
}
