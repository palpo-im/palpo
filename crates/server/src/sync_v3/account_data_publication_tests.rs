//! Commit-safe account data tested through real PostgreSQL and both HTTP sync APIs.
use std::time::Duration;

use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection};
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::core::{OwnedRoomId, OwnedUserId, UnixMillis, UserId};
use crate::{config, data};

const TOKEN: &str = "account-data-publication-token";
const LOCK_NAMESPACE: i64 = 0x50414c41434354;
const GATE_CLASS: i32 = 1_346_456_656;
const GATE_KEY: i32 = 2180;
const PREFIX: &str = "org.palpo.review.account_data.";

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

fn request(service: &Service, builder: RequestBuilder) -> JoinHandle<Value> {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = builder
            .add_header("Authorization", format!("Bearer {TOKEN}"), true)
            .send(router)
            .await;
        let body = response.take_json().await.unwrap();
        assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
        body
    })
}

async fn finish(task: JoinHandle<Value>) -> Value {
    tokio::time::timeout(Duration::from_secs(4), task)
        .await
        .expect("sync must finish after publication commits or rolls back")
        .unwrap()
}

fn sync(service: &Service, room: &OwnedRoomId, since: i64, sliding: bool) -> JoinHandle<Value> {
    if sliding {
        request(
            service,
            TestClient::post(format!(
                "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync?timeout=0&pos={since}"
            ))
            .json(&json!({
                "conn_id": "account-data",
                "room_subscriptions": {(room.as_str()): {"required_state": [], "timeline_limit": 1}},
                "extensions": {"account_data": {"enabled": true}}
            })),
        )
    } else {
        request(
            service,
            TestClient::get(format!(
                "http://localhost/_matrix/client/v3/sync?timeout=0&since={since}"
            )),
        )
    }
}

fn account_event<'a>(
    body: &'a Value,
    room: Option<&OwnedRoomId>,
    event_type: &str,
    sliding: bool,
) -> Option<&'a Value> {
    let events = match (sliding, room) {
        (false, None) => &body["account_data"]["events"],
        (false, Some(room)) => &body["rooms"]["join"][room.as_str()]["account_data"]["events"],
        (true, None) => &body["extensions"]["account_data"]["global"],
        (true, Some(room)) => &body["extensions"]["account_data"]["rooms"][room.as_str()],
    };
    events
        .as_array()?
        .iter()
        .find(|event| event["type"] == event_type)
}

async fn gate(conn: &mut AsyncPgConnection, hold: bool) {
    let query = if hold {
        "SELECT pg_advisory_lock($1, $2)"
    } else {
        "SELECT pg_advisory_unlock($1, $2)"
    };
    diesel::sql_query(query)
        .bind::<diesel::sql_types::Integer, _>(GATE_CLASS)
        .bind::<diesel::sql_types::Integer, _>(GATE_KEY)
        .execute(conn)
        .await
        .unwrap();
}

async fn wait_for_gate_writer() {
    wait_for("SELECT count(*) AS count FROM pg_locks WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND locktype='advisory' AND NOT granted AND classid=1346456656::oid AND objid=2180::oid AND objsubid=2").await;
}

async fn wait_for_snapshot(user: &UserId) {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let waiting = diesel::sql_query("SELECT count(*) AS count FROM pg_locks WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND locktype='advisory' AND NOT granted AND mode='ExclusiveLock' AND objsubid=1 AND classid::bigint=((hashtextextended($1,$2)>>32)&4294967295) AND objid::bigint=(hashtextextended($1,$2)&4294967295)")
                .bind::<diesel::sql_types::Text,_>(user.as_str())
                .bind::<diesel::sql_types::BigInt,_>(LOCK_NAMESPACE)
                .get_result::<Count>(&mut data::db()).await.unwrap();
            if waiting.count > 0 { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("HTTP sync must wait on account-data publication, not issue a cursor past it");
}

async fn wait_for(query: &str) {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if diesel::sql_query(query)
                .get_result::<Count>(&mut data::db())
                .await
                .unwrap()
                .count
                > 0
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("writer must reach the deterministic post-allocation database barrier");
}

#[derive(Clone, Copy, Debug)]
enum Mutation {
    Insert,
    Update,
    CompareAndSet,
    Delete,
}

async fn mutate(
    user: OwnedUserId,
    room: Option<OwnedRoomId>,
    event_type: String,
    mutation: Mutation,
) -> data::DataResult<()> {
    match mutation {
        Mutation::Insert | Mutation::Update => {
            data::user::set_data(&user, room, &event_type, json!({"value": "new"})).await?;
        }
        Mutation::CompareAndSet => {
            assert!(
                data::user::set_data_if_unchanged(
                    &user,
                    room,
                    &event_type,
                    Some(&json!({"value": "old"})),
                    json!({"value": "new"})
                )
                .await?
            );
        }
        Mutation::Delete => {
            if let Some(room) = room {
                data::user::delete_room_data(&user, &room, &event_type).await?;
            } else {
                data::user::delete_global_data(&user, &event_type).await?;
            }
        }
    }
    Ok(())
}

async fn setup() -> (Service, OwnedUserId, OwnedRoomId, AsyncPgConnection) {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name": "account-data.example", "db": {"url": "unused"},
                "auto_join_rooms": [],
                "presence": {"allow_local": false, "allow_incoming": false, "allow_outgoing": false}
            }))
            .unwrap(),
        )
        .unwrap();
    let user = UserId::parse("@reader:account-data.example").unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "reader".into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&user).await.unwrap();
    data::user::device::create_device(&user, "D".into(), TOKEN, None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&user).await.unwrap();
    let service = Service::new(crate::routing::root());
    let created = finish(request(
        &service,
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .json(&json!({"preset": "public_chat", "room_version": "11"})),
    ))
    .await;
    let room = created["room_id"].as_str().unwrap().parse().unwrap();
    let mut control =
        AsyncPgConnection::establish(&std::env::var("PALPO_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    diesel::sql_query("CREATE FUNCTION review_account_data_gate() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(1346456656,2180); IF NEW.data_type='org.palpo.review.account_data.rollback' THEN RAISE EXCEPTION 'injected account-data rollback'; END IF; RETURN NEW; END $$")
        .execute(&mut control).await.unwrap();
    diesel::sql_query("CREATE TRIGGER review_account_data_gate BEFORE INSERT OR UPDATE ON user_datas FOR EACH ROW WHEN (NEW.data_type LIKE 'org.palpo.review.account_data.%') EXECUTE FUNCTION review_account_data_gate()")
        .execute(&mut control).await.unwrap();
    (service, user, room, control)
}

async fn publication_contract() {
    let (service, user, room, mut control) = setup().await;
    for scoped in [false, true] {
        for (index, mutation) in [
            Mutation::Insert,
            Mutation::Update,
            Mutation::CompareAndSet,
            Mutation::Delete,
        ]
        .into_iter()
        .enumerate()
        {
            // Every writer path is checked against both actual HTTP sync APIs.
            for sliding in [false, true] {
                let event_type = format!("{PREFIX}{scoped}.{index}.{sliding}");
                let scope = scoped.then(|| room.clone());
                if !matches!(mutation, Mutation::Insert) {
                    data::user::set_data(&user, scope.clone(), &event_type, json!({"value":"old"}))
                        .await
                        .unwrap();
                }
                let since = data::curr_sn().await.unwrap() + 1;
                gate(&mut control, true).await;
                let writer = tokio::spawn(mutate(
                    user.clone(),
                    scope.clone(),
                    event_type.clone(),
                    mutation,
                ));
                wait_for_gate_writer().await;
                let reserved = data::curr_sn().await.unwrap();
                assert!(reserved >= since);
                // A different user can commit a higher global SN while this user
                // is paused. This is precisely when an unfenced cursor loses data.
                data::user::set_data(
                    crate::core::user_id!("@independent:account-data.example"),
                    None,
                    "org.palpo.independent",
                    json!({"position":reserved}),
                )
                .await
                .unwrap();
                let reader = sync(&service, &room, since, sliding);
                wait_for_snapshot(&user).await;
                assert!(!reader.is_finished());
                gate(&mut control, false).await;
                tokio::time::timeout(Duration::from_secs(4), writer)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                let body = finish(reader).await;
                let event = account_event(&body, scope.as_ref(), &event_type, sliding)
                    .unwrap_or_else(|| panic!("late {mutation:?} was skipped by sync: {body}"));
                assert_eq!(
                    event["content"],
                    if matches!(mutation, Mutation::Delete) {
                        json!({})
                    } else {
                        json!({"value":"new"})
                    }
                );
                let next = if sliding {
                    body["pos"]
                        .as_str()
                        .unwrap()
                        .parse::<crate::event::SyncPosition>()
                        .unwrap()
                        .event_sn
                } else {
                    body["next_batch"]
                        .as_str()
                        .unwrap()
                        .parse::<crate::event::BatchToken>()
                        .unwrap()
                        .event_sn()
                };
                assert!(next > reserved);
                let following = finish(sync(&service, &room, next, sliding)).await;
                assert!(account_event(&following, scope.as_ref(), &event_type, sliding).is_none());
            }
        }
    }
    for cancel in [false, true] {
        let event_type = format!("{PREFIX}{}", if cancel { "cancel" } else { "rollback" });
        let since = data::curr_sn().await.unwrap() + 1;
        gate(&mut control, true).await;
        let writer = tokio::spawn(mutate(
            user.clone(),
            None,
            event_type.clone(),
            Mutation::Insert,
        ));
        wait_for_gate_writer().await;
        let reader = sync(&service, &room, since, cancel);
        wait_for_snapshot(&user).await;
        if cancel {
            writer.abort();
            assert!(writer.await.unwrap_err().is_cancelled());
            // Aborting the Rust future discards its connection, but a busy PostgreSQL
            // backend may observe socket closure only after its current SQL finishes.
            // Release our artificial SQL barrier before requiring rollback: the sync
            // fence must wait for the actual transaction end, not just future Drop.
            // No checkout of the cancelled writer's pool is needed for cleanup.
            gate(&mut control, false).await;
            let body = finish(reader).await;
            assert!(account_event(&body, None, &event_type, cancel).is_none());
        } else {
            gate(&mut control, false).await;
            assert!(writer.await.unwrap().is_err());
            let body = finish(reader).await;
            assert!(account_event(&body, None, &event_type, cancel).is_none());
        }
        assert!(
            data::user::get_global_data::<Value>(&user, &event_type)
                .await
                .unwrap()
                .is_none()
        );
    }
    diesel::sql_query("DROP TRIGGER review_account_data_gate ON user_datas")
        .execute(&mut control)
        .await
        .unwrap();
    diesel::sql_query("DROP FUNCTION review_account_data_gate()")
        .execute(&mut control)
        .await
        .unwrap();
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_account_data_publication_fences_sync_and_releases_failures() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(publication_contract());
        })
        .unwrap()
        .join()
        .unwrap();
}
