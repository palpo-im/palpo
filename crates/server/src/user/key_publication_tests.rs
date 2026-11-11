//! Commit-safe device-list positions, exercised against a dedicated PostgreSQL.
use std::time::Duration;

use diesel::prelude::*;
use diesel_async::{AsyncConnection, SimpleAsyncConnection};
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedRoomId, OwnedUserId, Seqnum, UnixMillis, UserId};
use crate::data::schema::*;
use crate::{AppResult, config, data};

const STREAM_LOCK: i64 = 0x50414c4b455953;

#[derive(QueryableByName)]
struct Number {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    number: i64,
}

async fn wait_for_locks(class: i64, key: i64, sub: i16, count: i64) {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let waiting = diesel::sql_query("SELECT count(*) AS number FROM pg_locks WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND locktype='advisory' AND NOT granted AND classid::bigint=$1 AND objid::bigint=$2 AND objsubid=$3")
                .bind::<diesel::sql_types::BigInt,_>(class)
                .bind::<diesel::sql_types::BigInt,_>(key)
                .bind::<diesel::sql_types::SmallInt,_>(sub)
                .get_result::<Number>(&mut crate::data::db()).await.unwrap();
            if waiting.number >= count { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("writer and snapshot must reach the deterministic DB barriers");
}

async fn provision(name: &str) -> (OwnedUserId, String) {
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
    let token = format!("{name}-token");
    data::user::device::create_device(&user, "D".into(), &token, None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&user).await.unwrap();
    (user, token)
}

async fn get(service: &Service, token: &str, path: String) -> Value {
    let router = service.router();
    let token = token.to_owned();
    let mut response = tokio::spawn(async move {
        TestClient::get(format!("http://localhost/_matrix/client/v3/{path}"))
            .add_header("Authorization", format!("Bearer {token}"), true)
            .send(router)
            .await
    })
    .await
    .unwrap();
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

async fn sync(service: &Service, token: &str, since: Option<&str>) -> Value {
    get(
        service,
        token,
        format!(
            "sync?timeout=0{}",
            since.map_or(String::new(), |s| format!("&since={s}"))
        ),
    )
    .await
}

async fn room(service: &Service, token: &str) -> OwnedRoomId {
    let router = service.router();
    let token = token.to_owned();
    let mut response = tokio::spawn(async move {
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", format!("Bearer {token}"), true)
            .json(&json!({"preset":"public_chat","room_version":"11"}))
            .send(router)
            .await
    })
    .await
    .unwrap();
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body["room_id"].as_str().unwrap().parse().unwrap()
}

/// A trusted imported membership snapshot. This isolates remote-version
/// admission/publication; it does not pretend to verify a remote HTTP signature.
async fn imported_peer(room: &OwnedRoomId, peer: &OwnedUserId) {
    use crate::data::room::{DbEventData, NewDbEvent, NewDbRoomUser};
    use crate::room::state::{self, CompressedEvent};
    let before = crate::room::get_frame_id(room, None).await.unwrap();
    let sn = data::next_sn().await.unwrap();
    let pdu: crate::event::PduEvent = serde_json::from_value(json!({
        "event_id":"$key-fence-peer","room_id":room,"sender":peer,
        "type":"m.room.member","state_key":peer,"content":{"membership":"join"},
        "origin_server_ts":1,"depth":100,"hashes":{"sha256":""}
    }))
    .unwrap();
    let canonical = crate::core::serde::to_canonical_object(&pdu).unwrap();
    let mut event =
        NewDbEvent::from_canonical_json_with_room_id(&pdu.event_id, sn, &canonical, false, room)
            .unwrap();
    event.is_outlier = false;
    let mut conn = data::connect();
    event.save_with_conn(&mut conn).await.unwrap();
    DbEventData {
        event_id: pdu.event_id.clone(),
        event_sn: sn,
        room_id: room.clone(),
        json_data: serde_json::to_value(&canonical).unwrap(),
        internal_metadata: None,
        format_version: None,
    }
    .save_with_conn(&mut conn)
    .await
    .unwrap();
    diesel::insert_into(event_points::table)
        .values((
            event_points::event_id.eq(&pdu.event_id),
            event_points::event_sn.eq(sn),
            event_points::room_id.eq(room),
            event_points::before_frame_id.eq(before),
        ))
        .execute(&mut conn)
        .await
        .unwrap();
    drop(conn);
    let mut full = state::load_frame_info(before)
        .await
        .unwrap()
        .last()
        .unwrap()
        .full_state
        .as_ref()
        .clone();
    let field = state::ensure_field_id(&"m.room.member".into(), peer.as_str())
        .await
        .unwrap();
    full.insert(CompressedEvent::new(field, sn));
    let frame = state::save_state(room, std::sync::Arc::new(full))
        .await
        .unwrap()
        .frame_id;
    state::set_room_state(room, frame).await.unwrap();
    let mut conn = data::connect();
    diesel::insert_into(room_users::table)
        .values(NewDbRoomUser {
            event_id: pdu.event_id.clone(),
            event_sn: sn,
            room_id: room.clone(),
            room_server_id: None,
            user_id: peer.clone(),
            user_server_id: peer.server_name().to_owned(),
            sender_id: peer.clone(),
            membership: "join".into(),
            forgotten: false,
            display_name: None,
            avatar_url: None,
            state_data: None,
            created_at: UnixMillis::now(),
        })
        .execute(&mut conn)
        .await
        .unwrap();
    let pdu = crate::SnPduEvent::new(pdu, sn, false, false, false);
    conn.transaction::<_, crate::AppError, _>(async |conn| {
        crate::event::sticky::publish_live_frame_with_conn(conn, &pdu, frame).await
    })
    .await
    .unwrap();
}

async fn publish(kind: u8, user: &UserId, room: &OwnedRoomId, version: u64) -> AppResult<()> {
    match kind {
        0 => crate::user::mark_device_key_update(user, "D".into()).await?,
        1 => crate::user::mark_signing_key_update(user).await?,
        2 => {
            crate::user::mark_device_key_update_with_joined_rooms(
                user,
                "D".into(),
                std::slice::from_ref(room),
            )
            .await?
        }
        3 => assert!(
            data::user::key::apply_remote_device_list_version(user, version, config::server_name())
                .await?
        ),
        4 => {
            data::user::device::remove_other_devices(
                user,
                "D".into(),
                std::slice::from_ref(room),
                &[],
            )
            .await?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn captures(reader: &OwnedUserId) -> Vec<tokio::task::JoinHandle<i64>> {
    (0..3)
        .map(|kind| {
            let user = reader.clone();
            tokio::spawn(async move {
                let device: crate::core::OwnedDeviceId = format!("CAP{kind}").into();
                match kind {
                    0 => crate::event::sticky::curr_sn_after_sync_writes(&user, &device)
                        .await
                        .unwrap(),
                    1 => data::user::curr_sn_after_presence_writes(Some((&user, &device)))
                        .await
                        .unwrap(),
                    _ => {
                        data::user::curr_sn_after_presence_profile_and_inbox_writes(&user, &device)
                            .await
                            .unwrap()
                    }
                }
            })
        })
        .collect()
}

async fn assert_notifications(
    service: &Service,
    reader: &OwnedUserId,
    token: &str,
    from: &str,
    changed: &[OwnedUserId],
) {
    let response = sync(service, token, Some(from)).await;
    let lists: crate::core::device::DeviceLists = serde_json::from_value(
        response
            .get("device_lists")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .unwrap();
    assert_eq!(lists.changed, changed, "{response}");
    let keys = get(
        service,
        token,
        format!(
            "keys/changes?from={from}&to={}",
            response["next_batch"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(
        keys["changed"],
        serde_json::to_value(changed).unwrap(),
        "{keys}"
    );
    let request = serde_json::from_value(json!({"extensions":{"e2ee":{"enabled":true}}})).unwrap();
    let since = from.parse::<crate::event::BatchToken>().unwrap().event_sn();
    let sliding =
        crate::sync_v5::sync_events(reader, "D".into(), since, &request, &Default::default())
            .await
            .unwrap();
    assert_eq!(sliding.extensions.e2ee.device_lists.changed, changed);
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn review_key_change_publication_fence_survives_commit_rollback_and_cancellation() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn run() {
    crate::test_database::init();
    config::CONFIG.set(serde_json::from_value(json!({
        "server_name":"key-fence.example","db":{"url":"unused-test-config"},"ip_range_denylist":[],"auto_join_rooms":[],
        "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
    })).unwrap()).unwrap();
    let (reader, token) = provision("alice").await;
    let peer = UserId::parse("@bob:remote-key-fence.example").unwrap();
    let service = Service::new(crate::routing::root());
    let room = room(&service, &token).await;
    imported_peer(&room, &peer).await;
    let mut control = data::checkout().await.unwrap();
    control.batch_execute("CREATE TABLE key_fence_control(target_user TEXT, fail BOOLEAN); INSERT INTO key_fence_control VALUES ('',false);
        CREATE FUNCTION key_fence_pause() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
        IF NEW.user_id=(SELECT target_user FROM key_fence_control) THEN
          PERFORM pg_advisory_xact_lock(712,491);
          IF (SELECT fail FROM key_fence_control) THEN RAISE EXCEPTION 'key fence rollback'; END IF;
        END IF; RETURN NEW; END $$;
        CREATE TRIGGER zz_key_fence_pause BEFORE INSERT ON e2e_key_changes FOR EACH ROW EXECUTE FUNCTION key_fence_pause();").await.unwrap();
    drop(control);
    for kind in 0..5 {
        for fail in [false, true] {
            let user = if kind == 3 {
                peer.clone()
            } else {
                reader.clone()
            };
            if kind == 4
                && data::user::device::get_device(&reader, "DELETE".into())
                    .await
                    .is_err()
            {
                data::user::device::create_device(
                    &reader,
                    "DELETE".into(),
                    "key-fence-delete",
                    None,
                    None,
                )
                .await
                .unwrap();
            }
            let baseline = sync(&service, &token, None).await;
            let from = baseline["next_batch"].as_str().unwrap().to_owned();
            let mut gate = data::checkout().await.unwrap();
            diesel::sql_query("UPDATE key_fence_control SET target_user=$1,fail=$2")
                .bind::<diesel::sql_types::Text, _>(user.as_str())
                .bind::<diesel::sql_types::Bool, _>(fail)
                .execute(&mut gate)
                .await
                .unwrap();
            gate.batch_execute("BEGIN; SELECT pg_advisory_xact_lock(712,491)")
                .await
                .unwrap();
            let writing_user = user.clone();
            let writing_room = room.clone();
            let version = 100 + u64::from(kind) * 2 + u64::from(fail);
            let writer =
                tokio::spawn(
                    async move { publish(kind, &writing_user, &writing_room, version).await },
                );
            wait_for_locks(712, 491, 2, 1).await;
            let reserved = data::curr_sn().await.unwrap();
            let snapshotters = captures(&reader);
            wait_for_locks(STREAM_LOCK >> 32, STREAM_LOCK & 0xffff_ffff, 1, 3).await;
            for snapshot in &snapshotters {
                assert!(
                    !snapshot.is_finished(),
                    "cursor escaped an uncommitted key position"
                );
            }
            gate.batch_execute("COMMIT").await.unwrap();
            drop(gate);
            let result = tokio::time::timeout(Duration::from_secs(4), writer)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result.is_err(), fail, "kind={kind}, error={result:?}");
            for snapshot in snapshotters {
                let captured = tokio::time::timeout(Duration::from_secs(4), snapshot)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(captured >= reserved);
            }
            let expected = if fail { vec![] } else { vec![user.clone()] };
            assert_notifications(&service, &reader, &token, &from, &expected).await;
            if fail {
                if kind == 4 {
                    assert!(
                        data::user::device::get_device(&reader, "DELETE".into())
                            .await
                            .is_ok(),
                        "device revocation failed to roll back"
                    );
                }
                diesel::sql_query("UPDATE key_fence_control SET fail=false")
                    .execute(&mut crate::data::db())
                    .await
                    .unwrap();
                tokio::time::timeout(Duration::from_secs(4), publish(kind, &user, &room, version))
                    .await
                    .unwrap()
                    .unwrap();
                assert_notifications(
                    &service,
                    &reader,
                    &token,
                    &from,
                    std::slice::from_ref(&user),
                )
                .await;
            }
        }
    }
    cancellation_and_concurrency(&service, &reader, &peer, &token, &room).await;
}

async fn cancellation_and_concurrency(
    service: &Service,
    reader: &OwnedUserId,
    peer: &OwnedUserId,
    token: &str,
    room: &OwnedRoomId,
) {
    diesel::sql_query("UPDATE key_fence_control SET target_user='',fail=false")
        .execute(&mut crate::data::db())
        .await
        .unwrap();
    let (started, waiting) = tokio::sync::oneshot::channel();
    let user = reader.clone();
    let cancelled = tokio::spawn(async move {
        let mut conn = data::checkout().await.unwrap();
        conn.transaction::<_, data::DataError, _>(async |conn| {
            data::user::key::lock_key_change_user(conn, &user).await?;
            data::user::key::lock_key_change_stream(conn).await?;
            let pid = diesel::sql_query("SELECT pg_backend_pid()::bigint AS number")
                .get_result::<Number>(conn)
                .await?
                .number;
            let position = diesel::sql_query("SELECT nextval('occur_sn_seq') AS number")
                .get_result::<Number>(conn)
                .await?
                .number;
            diesel::insert_into(e2e_key_changes::table)
                .values(data::user::NewDbKeyChange {
                    user_id: user.clone(),
                    room_id: None,
                    occur_sn: position,
                    changed_at: UnixMillis::now(),
                })
                .on_conflict((e2e_key_changes::user_id, e2e_key_changes::room_id))
                .do_update()
                .set(e2e_key_changes::occur_sn.eq(position))
                .execute(conn)
                .await?;
            started.send((pid, position)).unwrap();
            // Real SQL has completed, but this transaction's future is dropped
            // before commit. No test rollback or explicit backend cancellation
            // is performed: the pool must discard this dirty connection.
            std::future::pending::<()>().await;
            Ok(())
        })
        .await
    });
    let (cancelled_pid, cancelled_sn) = waiting.await.unwrap();
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    let mut leased = Vec::new();
    for _ in 0..data::status().max_size {
        let mut conn = data::checkout().await.unwrap();
        let pid = diesel::sql_query("SELECT pg_backend_pid()::bigint AS number")
            .get_result::<Number>(&mut conn)
            .await
            .unwrap()
            .number;
        assert_ne!(
            pid, cancelled_pid,
            "pool recycled a connection carrying an open key transaction"
        );
        leased.push(conn);
    }
    drop(leased);
    for captured in captures(reader) {
        tokio::time::timeout(Duration::from_secs(4), captured)
            .await
            .unwrap()
            .unwrap();
    }
    assert!(
        data::user::key::keys_changed_in_window(
            std::slice::from_ref(reader),
            cancelled_sn,
            cancelled_sn + 1
        )
        .await
        .unwrap()
        .is_empty(),
        "cancellation committed an unacknowledged journal row"
    );
    let from = format!("p{}", cancelled_sn + 1);
    tokio::time::timeout(
        Duration::from_secs(4),
        data::user::key::publish_key_change(reader, std::slice::from_ref(room), true),
    )
    .await
    .unwrap()
    .unwrap();
    assert_notifications(service, reader, token, &from, std::slice::from_ref(reader)).await;

    let baseline = sync(service, token, None).await;
    let from = baseline["next_batch"].as_str().unwrap().to_owned();
    let lower = from.parse::<crate::event::BatchToken>().unwrap().event_sn();
    let mut gate = data::checkout().await.unwrap();
    diesel::sql_query("UPDATE key_fence_control SET target_user=$1")
        .bind::<diesel::sql_types::Text, _>(reader.as_str())
        .execute(&mut gate)
        .await
        .unwrap();
    gate.batch_execute("BEGIN; SELECT pg_advisory_xact_lock(712,491)")
        .await
        .unwrap();
    let first_user = reader.clone();
    let first_room = room.clone();
    let first = tokio::spawn(async move {
        data::user::key::publish_key_change(&first_user, &[first_room.clone(), first_room], true)
            .await
    });
    wait_for_locks(712, 491, 2, 1).await;
    let reserved = data::curr_sn().await.unwrap();
    let second_user = reader.clone();
    let second_room = room.clone();
    let second = tokio::spawn(async move {
        data::user::key::publish_key_change(&second_user, &[second_room], true).await
    });
    tokio::time::timeout(Duration::from_secs(4),async {
        loop {
            let waiting=diesel::sql_query("SELECT count(*) AS number FROM pg_locks WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND locktype='advisory' AND NOT granted AND objsubid=1 AND classid::bigint=((hashtextextended($1,$2)>>32)&4294967295) AND objid::bigint=(hashtextextended($1,$2)&4294967295)")
                .bind::<diesel::sql_types::Text,_>(reader.as_str()).bind::<diesel::sql_types::BigInt,_>(1_757_253_914_i64)
                .get_result::<Number>(&mut crate::data::db()).await.unwrap();
            if waiting.number>0 {break;}
            tokio::task::yield_now().await;
        }
    }).await.expect("second same-user publisher must wait before allocating another SN");
    assert_eq!(data::curr_sn().await.unwrap(), reserved);
    tokio::time::timeout(
        Duration::from_secs(4),
        data::user::key::publish_key_change(peer, std::slice::from_ref(room), true),
    )
    .await
    .unwrap()
    .unwrap();
    let snapshotters = captures(reader);
    wait_for_locks(STREAM_LOCK >> 32, STREAM_LOCK & 0xffff_ffff, 1, 3).await;
    gate.batch_execute("COMMIT").await.unwrap();
    drop(gate);
    tokio::time::timeout(Duration::from_secs(4), first)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(4), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    for snapshot in snapshotters {
        tokio::time::timeout(Duration::from_secs(4), snapshot)
            .await
            .unwrap()
            .unwrap();
    }
    let count = diesel::sql_query(
        "SELECT count(*) AS number FROM e2e_key_change_history WHERE user_id=$1 AND occur_sn >= $2",
    )
    .bind::<diesel::sql_types::Text, _>(reader.as_str())
    .bind::<diesel::sql_types::BigInt, _>(lower)
    .get_result::<Number>(&mut crate::data::db())
    .await
    .unwrap();
    assert_eq!(
        count.number, 2,
        "same-user publishers must both publish exactly one journal position"
    );
    let last = diesel::sql_query(
        "SELECT max(occur_sn) AS number FROM e2e_key_change_history WHERE user_id=$1",
    )
    .bind::<diesel::sql_types::Text, _>(reader.as_str())
    .get_result::<Number>(&mut crate::data::db())
    .await
    .unwrap()
    .number;
    let marker = e2e_key_changes::table
        .filter(e2e_key_changes::user_id.eq(reader))
        .filter(e2e_key_changes::room_id.is_null())
        .select(e2e_key_changes::occur_sn)
        .first::<Seqnum>(&mut crate::data::db())
        .await
        .unwrap();
    assert_eq!(
        marker, last,
        "latest scope marker regressed during concurrent publication"
    );
    assert_notifications(
        service,
        reader,
        token,
        &from,
        &[reader.clone(), peer.clone()],
    )
    .await;
}
