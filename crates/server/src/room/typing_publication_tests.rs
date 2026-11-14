//! Typing publications and sync cursors must agree across PostgreSQL connections.
use std::time::Duration;

use diesel::prelude::*;
use diesel_async::SimpleAsyncConnection;
use horme::RunQueryDsl;

use crate::core::{OwnedRoomId, OwnedUserId, UnixMillis};
use crate::data;

const STREAM_LOCK: i64 = 0x50414c54595045;

#[derive(QueryableByName)]
struct Number {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    number: i64,
}

async fn wait_for_locks(class: i64, key: i64, sub: i16, count: i64) {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let waiting = diesel::sql_query(
                "SELECT count(*) AS number FROM pg_locks
                 WHERE database=(SELECT oid FROM pg_database WHERE datname=current_database())
                 AND locktype='advisory' AND NOT granted
                 AND classid::bigint=$1 AND objid::bigint=$2 AND objsubid=$3",
            )
            .bind::<diesel::sql_types::BigInt, _>(class)
            .bind::<diesel::sql_types::BigInt, _>(key)
            .bind::<diesel::sql_types::SmallInt, _>(sub)
            .get_result::<Number>(&mut data::db())
            .await
            .unwrap();
            if waiting.number >= count {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("typing writers and sync snapshots must reach the database barriers");
}

fn capture_positions(user: &OwnedUserId) -> Vec<tokio::task::JoinHandle<i64>> {
    (0..3)
        .map(|kind| {
            let user = user.clone();
            tokio::spawn(async move {
                let device: crate::core::OwnedDeviceId = format!("TYPING{kind}").into();
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

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
async fn database_typing_publication_fences_sync_until_commit_or_rollback() {
    crate::test_database::init();
    let room: OwnedRoomId = "!typing-fence:example.org".try_into().unwrap();
    let user: OwnedUserId = "@typing-fence:example.org".try_into().unwrap();
    let mut control = data::checkout().await.unwrap();
    control
        .batch_execute(
            "CREATE TABLE typing_fence_control(target_room TEXT, fail BOOLEAN);
             INSERT INTO typing_fence_control VALUES ('',false);
             CREATE FUNCTION typing_fence_pause() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
               IF NEW.room_id=(SELECT target_room FROM typing_fence_control) THEN
                 PERFORM pg_advisory_xact_lock(713,662);
                 IF (SELECT fail FROM typing_fence_control) THEN
                   RAISE EXCEPTION 'typing publication rollback';
                 END IF;
               END IF;
               RETURN NEW;
             END $$;
             CREATE TRIGGER typing_fence_pause AFTER INSERT OR UPDATE ON room_typings
             FOR EACH ROW EXECUTE FUNCTION typing_fence_pause();",
        )
        .await
        .unwrap();
    drop(control);

    // Exercise every typing publisher with both a committed and rolled-back SN.
    for kind in 0..4 {
        for fail in [false, true] {
            diesel::sql_query("UPDATE typing_fence_control SET target_room='',fail=false")
                .execute(&mut data::db())
                .await
                .unwrap();
            let now = UnixMillis::now().get() as i64;
            let deadline = if kind == 2 { now - 1 } else { now + 60_000 };
            let baseline = data::room::typing::upsert_typing(&room, &user, deadline)
                .await
                .unwrap();
            let mut gate = data::checkout().await.unwrap();
            diesel::sql_query("UPDATE typing_fence_control SET target_room=$1,fail=$2")
                .bind::<diesel::sql_types::Text, _>(room.as_str())
                .bind::<diesel::sql_types::Bool, _>(fail)
                .execute(&mut gate)
                .await
                .unwrap();
            gate.batch_execute("BEGIN; SELECT pg_advisory_xact_lock(713,662)")
                .await
                .unwrap();
            let writing_room = room.clone();
            let writing_user = user.clone();
            let writer = tokio::spawn(async move {
                match kind {
                    0 => data::room::typing::upsert_typing(
                        &writing_room,
                        &writing_user,
                        now + 90_000,
                    )
                    .await
                    .map(|_| ()),
                    1 => data::room::typing::stop_typing(&writing_room, &writing_user, now).await,
                    2 => data::room::typing::publish_expired_typings(now)
                        .await
                        .map(|_| ()),
                    _ => data::room::typing::clamp_typing_deadlines(now + 30_000)
                        .await
                        .map(|_| ()),
                }
            });
            // The trigger pauses real production SQL after nextval, before commit.
            wait_for_locks(713, 662, 2, 1).await;
            let reserved = data::curr_sn().await.unwrap();
            assert!(reserved > baseline);
            assert_eq!(super::last_typing_update(&room).await.unwrap(), baseline);
            let snapshots = capture_positions(&user);
            wait_for_locks(STREAM_LOCK >> 32, STREAM_LOCK & 0xffff_ffff, 1, 3).await;
            for snapshot in &snapshots {
                assert!(
                    !snapshot.is_finished(),
                    "sync skipped an uncommitted typing SN"
                );
            }
            gate.batch_execute("COMMIT").await.unwrap();
            drop(gate);
            let result = tokio::time::timeout(Duration::from_secs(4), writer)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(result.is_err(), fail, "kind={kind}, result={result:?}");
            let published = super::last_typing_update(&room).await.unwrap();
            if fail {
                assert_eq!(published, baseline);
            } else {
                assert!(published > baseline && published <= reserved);
            }
            for snapshot in snapshots {
                let captured = tokio::time::timeout(Duration::from_secs(4), snapshot)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(captured >= reserved);
            }
            if !fail {
                assert!(
                    published > baseline,
                    "the next incremental sync must see the change"
                );
            }
        }
    }
    data::checkout()
        .await
        .unwrap()
        .batch_execute(
            "DROP TRIGGER typing_fence_pause ON room_typings;
             DROP FUNCTION typing_fence_pause(); DROP TABLE typing_fence_control;",
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
async fn database_typing_delayed_expiry_retains_full_publication_grace() {
    crate::test_database::init();
    let room: OwnedRoomId = "!typing-delayed-expiry:example.org".try_into().unwrap();
    let user: OwnedUserId = "@typing-delayed-expiry:example.org".try_into().unwrap();
    let now = UnixMillis::now().get() as i64;
    let grace = data::room::typing::TYPING_ROW_GRACE_MS;
    let before = data::room::typing::upsert_typing(&room, &user, now - 2 * grace)
        .await
        .unwrap();

    // A sync that runs before a delayed startup sweep must not erase its work.
    assert!(
        super::all_typings(&room)
            .await
            .unwrap()
            .content
            .user_ids
            .is_empty()
    );
    assert_eq!(super::last_typing_update(&room).await.unwrap(), before);
    data::room::typing::publish_expired_typings(now)
        .await
        .unwrap();
    let published = super::last_typing_update(&room).await.unwrap();
    assert!(published > before);
    // Two clients can both observe the newly published empty typing list.
    for _ in 0..2 {
        assert!(super::last_typing_update(&room).await.unwrap() > before);
        assert!(
            super::all_typings(&room)
                .await
                .unwrap()
                .content
                .user_ids
                .is_empty()
        );
    }
    for offset in [0, grace - 1] {
        data::room::typing::delete_expired_typings(&room, now + offset)
            .await
            .unwrap();
        assert_eq!(super::last_typing_update(&room).await.unwrap(), published);
    }
    data::room::typing::publish_expired_typings(now + grace - 1)
        .await
        .unwrap();
    assert_eq!(super::last_typing_update(&room).await.unwrap(), published);
    data::room::typing::delete_expired_typings(&room, now + grace + 1)
        .await
        .unwrap();
    assert_eq!(super::last_typing_update(&room).await.unwrap(), 0);
}
