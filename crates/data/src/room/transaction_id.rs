use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::core::identifiers::*;
use crate::core::{DeviceId, TransactionId, UnixMillis, UserId};
use crate::room::NewDbEventIdempotent;
use crate::schema::*;
use crate::{DataResult, connect};

/// Record a transaction id together with the event it produced for idempotency.
pub async fn add_txn_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
    event_id: Option<&EventId>,
) -> DataResult<()> {
    add_scoped_txn_id(txn_id, user_id, device_id, room_id, event_id, "").await
}

pub async fn add_scoped_txn_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
    event_id: Option<&EventId>,
    endpoint: &str,
) -> DataResult<()> {
    diesel::insert_into(event_idempotents::table)
        .values(&NewDbEventIdempotent {
            txn_id: txn_id.to_owned(),
            user_id: user_id.to_owned(),
            device_id: device_id.map(|d| d.to_owned()),
            room_id: room_id.map(|r| r.to_owned()),
            event_id: event_id.map(|e| e.to_owned()),
            created_at: UnixMillis::now(),
            endpoint: endpoint.to_owned(),
        })
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Check whether a `(user, device, txn_id)` combination has already been seen.
pub async fn txn_id_exists(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
) -> DataResult<bool> {
    if let Some(device_id) = device_id {
        let query = event_idempotents::table
            .filter(event_idempotents::endpoint.eq(""))
            .filter(event_idempotents::user_id.eq(user_id))
            .filter(event_idempotents::device_id.eq(device_id))
            .filter(event_idempotents::txn_id.eq(txn_id))
            .select(event_idempotents::event_id);
        diesel_exists!(query, &mut connect().await?).map_err(Into::into)
    } else {
        let query = event_idempotents::table
            .filter(event_idempotents::endpoint.eq(""))
            .filter(event_idempotents::user_id.eq(user_id))
            .filter(event_idempotents::device_id.is_null())
            .filter(event_idempotents::txn_id.eq(txn_id))
            .select(event_idempotents::event_id);
        diesel_exists!(query, &mut connect().await?).map_err(Into::into)
    }
}

/// Look up the event id previously recorded for a transaction.
pub async fn get_event_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
) -> DataResult<Option<OwnedEventId>> {
    get_scoped_event_id(txn_id, user_id, device_id, room_id, "").await
}

pub async fn get_scoped_event_id(
    txn_id: &TransactionId,
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: Option<&RoomId>,
    endpoint: &str,
) -> DataResult<Option<OwnedEventId>> {
    let mut query = event_idempotents::table
        .filter(event_idempotents::endpoint.eq(endpoint))
        .filter(event_idempotents::user_id.eq(user_id))
        .filter(event_idempotents::txn_id.eq(txn_id))
        .into_boxed();
    if let Some(device_id) = device_id {
        query = query.filter(event_idempotents::device_id.eq(device_id));
    } else {
        query = query.filter(event_idempotents::device_id.is_null());
    }
    if let Some(room_id) = room_id {
        query = query.filter(event_idempotents::room_id.eq(room_id));
    } else {
        query = query.filter(event_idempotents::room_id.is_null());
    }
    query
        .select(event_idempotents::event_id)
        .first::<Option<OwnedEventId>>(&mut connect().await?)
        .await
        .optional()
        .map(|v| v.flatten())
        .map_err(Into::into)
}

/// Recover the originating device using the complete, locally recorded event identity.
pub async fn get_event_device(
    txn_id: &TransactionId,
    user_id: &UserId,
    room_id: &RoomId,
    event_id: &EventId,
) -> DataResult<Option<OwnedDeviceId>> {
    // New events save trusted provenance in the same transaction as their JSON,
    // before /sync can see them and before the route records idempotency completion.
    let metadata = event_datas::table
        .filter(event_datas::event_id.eq(event_id))
        .filter(event_datas::room_id.eq(room_id))
        .select(event_datas::internal_metadata)
        .first::<Option<serde_json::Value>>(&mut connect().await?)
        .await
        .optional()?
        .flatten();
    if let Some(metadata) = metadata
        && metadata
            .get("transaction_user")
            .and_then(serde_json::Value::as_str)
            == Some(user_id.as_str())
        && metadata
            .get("transaction_id")
            .and_then(serde_json::Value::as_str)
            == Some(txn_id.as_str())
        && let Some(device) = metadata
            .get("transaction_device")
            .and_then(serde_json::Value::as_str)
    {
        return Ok(Some(device.into()));
    }
    // Existing events predate the provenance field.
    event_idempotents::table
        .filter(event_idempotents::txn_id.eq(txn_id))
        .filter(event_idempotents::user_id.eq(user_id))
        .filter(event_idempotents::room_id.eq(room_id))
        .filter(event_idempotents::event_id.eq(event_id))
        .select(event_idempotents::device_id)
        .first::<Option<OwnedDeviceId>>(&mut connect().await?)
        .await
        .optional()
        .map(|v| v.flatten())
        .map_err(Into::into)
}

/// Commit all effects of a client to-device send together with its endpoint-scoped
/// receipt. The unique insert serializes competing retries across processes.
pub async fn commit_to_device_transaction(
    receipt: &NewDbEventIdempotent,
    inboxes: &[crate::user::device::NewDbDeviceInbox],
    outgoing: &[crate::sending::NewDbOutgoingRequest],
) -> DataResult<bool> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            let inserted = diesel::insert_into(event_idempotents::table)
                .values(receipt)
                .on_conflict_do_nothing()
                .execute(conn)
                .await?;
            if inserted == 0 {
                return Ok(false);
            }
            // Small sends retain independent inbox concurrency. A large send
            // holds one exclusive batch fence rather than exhausting the
            // shared advisory-lock table with one lock per recipient.
            let targets: std::collections::BTreeSet<_> = inboxes
                .iter()
                .map(|inbox| (&*inbox.user_id, &*inbox.device_id))
                .collect();
            if targets.len() > 32 {
                crate::user::device::lock_inbox_batch_stream(conn).await?;
            } else {
                for (user, device) in targets {
                    crate::user::device::lock_inbox_stream(conn, user, device).await?;
                }
            }
            // PostgreSQL permits at most 65,535 bind parameters per statement.
            // These rows use four/seven columns; leave ample headroom while
            // retaining one transaction for every chunk and the receipt.
            for chunk in inboxes.chunks(1_000) {
                diesel::insert_into(device_inboxes::table)
                    .values(chunk)
                    .execute(conn)
                    .await?;
            }
            for chunk in outgoing.chunks(1_000) {
                diesel::insert_into(outgoing_requests::table)
                    .values(chunk)
                    .execute(conn)
                    .await?;
            }
            Ok(true)
        })
        .await
}

#[cfg(test)]
mod bulk_tests {
    use super::*;
    use crate::sending::NewDbOutgoingRequest;
    use crate::user::device::NewDbDeviceInbox;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; explicitly executed in CI"]
    async fn database_large_to_device_transaction_chunks_and_rolls_back() {
        let config = serde_json::from_value(serde_json::json!({
            "url": std::env::var("PALPO_TEST_DATABASE_URL").expect("dedicated database required")
        }))
        .unwrap();
        crate::init(&config).unwrap();
        async fn counts() -> (i64, i64, i64) {
            let mut conn = connect().await.unwrap();
            (
                device_inboxes::table
                    .count()
                    .get_result(&mut conn)
                    .await
                    .unwrap(),
                outgoing_requests::table
                    .count()
                    .get_result(&mut conn)
                    .await
                    .unwrap(),
                event_idempotents::table
                    .count()
                    .get_result(&mut conn)
                    .await
                    .unwrap(),
            )
        }
        assert_eq!(counts().await, (0, 0, 0), "fixture needs a fresh database");
        let user: OwnedUserId = "@bulk:transaction.example".try_into().unwrap();
        // Exercise real distinct targets under PostgreSQL's ordinary advisory
        // lock capacity as well as its wire-protocol parameter limit.
        let inboxes: Vec<_> = (0..17_000)
            .map(|index| NewDbDeviceInbox {
                user_id: user.clone(),
                device_id: format!("D{index}").into(),
                json_data: serde_json::json!({"index": index}),
                created_at: 0,
            })
            .collect();
        let outgoing: Vec<_> = (0..23_000)
            .map(|index| {
                NewDbOutgoingRequest {
                kind: "normal".into(),
                appservice_id: None,
                user_id: None,
                pushkey: None,
                server_id: Some("remote.example".try_into().unwrap()),
                pdu_id: None,
                edu_json: Some(serde_json::to_vec(&serde_json::json!({
                    "edu_type": "m.direct_to_device", "content": {"message_id": index.to_string()}
                })).unwrap()),
            }
            })
            .collect();
        let receipt = NewDbEventIdempotent {
            txn_id: "bulk-first".into(),
            user_id: user,
            device_id: Some("D".into()),
            room_id: None,
            event_id: None,
            created_at: UnixMillis::now(),
            endpoint: "to_device".into(),
        };
        assert!(
            commit_to_device_transaction(&receipt, &inboxes, &outgoing)
                .await
                .unwrap()
        );
        assert_eq!(counts().await, (17_000, 23_000, 1));
        assert!(
            !commit_to_device_transaction(&receipt, &inboxes, &outgoing)
                .await
                .unwrap()
        );
        assert_eq!(counts().await, (17_000, 23_000, 1));

        diesel::sql_query("ALTER TABLE outgoing_requests ADD CONSTRAINT reject_late_chunk CHECK (kind <> 'reject_late_chunk')")
            .execute(&mut connect().await.unwrap()).await.unwrap();
        let mut second_receipt = receipt;
        second_receipt.txn_id = "bulk-retry".into();
        let mut failed_outgoing = outgoing;
        failed_outgoing.last_mut().unwrap().kind = "reject_late_chunk".into();
        assert!(
            commit_to_device_transaction(&second_receipt, &inboxes, &failed_outgoing)
                .await
                .is_err()
        );
        assert_eq!(
            counts().await,
            (17_000, 23_000, 1),
            "a late outgoing chunk must roll back every inbox, outgoing chunk and receipt"
        );
        failed_outgoing.last_mut().unwrap().kind = "normal".into();
        assert!(
            commit_to_device_transaction(&second_receipt, &inboxes, &failed_outgoing)
                .await
                .unwrap()
        );
        assert_eq!(counts().await, (34_000, 46_000, 2));
        assert!(
            !commit_to_device_transaction(&second_receipt, &inboxes, &failed_outgoing)
                .await
                .unwrap()
        );
        assert_eq!(counts().await, (34_000, 46_000, 2));

        #[derive(diesel::QueryableByName)]
        struct Waiting {
            #[diesel(sql_type = diesel::sql_types::Bool)]
            waiting: bool,
        }
        async fn wait_for_lock(slot: i32, mode: &str) {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let waiting = diesel::sql_query("SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND classid=1346456656::oid AND objid=$1::oid AND objsubid=2 AND mode=$2 AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())) AS waiting")
                        .bind::<diesel::sql_types::Integer,_>(slot)
                        .bind::<diesel::sql_types::Text,_>(mode)
                        .get_result::<Waiting>(&mut connect().await.unwrap()).await.unwrap();
                    if waiting.waiting { break; }
                    tokio::task::yield_now().await;
                }
            }).await.expect("expected database lock waiter");
        }
        let mut probe_receipt = second_receipt.clone();
        probe_receipt.txn_id = "reader-before-batch".into();
        let probe_inboxes = inboxes[..64].to_vec();
        let probe_user = probe_inboxes[0].user_id.clone();
        let probe_device = probe_inboxes[0].device_id.clone();
        let mut reader_conn = connect().await.unwrap();
        diesel::sql_query("BEGIN")
            .execute(&mut reader_conn)
            .await
            .unwrap();
        crate::user::device::lock_inbox_stream(&mut reader_conn, &probe_user, &probe_device)
            .await
            .unwrap();
        let before = crate::curr_sn().await.unwrap();
        let writer = tokio::spawn(async move {
            commit_to_device_transaction(&probe_receipt, &probe_inboxes, &[]).await
        });
        wait_for_lock(0, "ExclusiveLock").await;
        assert_eq!(
            crate::curr_sn().await.unwrap(),
            before,
            "blocked batch cannot allocate positions before the reader completes"
        );
        diesel::sql_query("COMMIT")
            .execute(&mut reader_conn)
            .await
            .unwrap();
        assert!(writer.await.unwrap().unwrap());

        // Pause a real insertion after allocation but before commit. A sync
        // must wait even though the large writer holds no individual locks.
        diesel::sql_query("CREATE FUNCTION pause_bulk_insert() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.json_data->>'barrier'='true' THEN PERFORM pg_advisory_xact_lock(1346456656,17); END IF; RETURN NEW; END $$")
            .execute(&mut connect().await.unwrap()).await.unwrap();
        diesel::sql_query("CREATE TRIGGER pause_bulk_insert BEFORE INSERT ON device_inboxes FOR EACH ROW EXECUTE FUNCTION pause_bulk_insert()")
            .execute(&mut connect().await.unwrap()).await.unwrap();
        let mut controller = connect().await.unwrap();
        diesel::sql_query("BEGIN")
            .execute(&mut controller)
            .await
            .unwrap();
        diesel::sql_query("SELECT pg_advisory_xact_lock(1346456656,17)")
            .execute(&mut controller)
            .await
            .unwrap();
        let mut probe_receipt = second_receipt.clone();
        probe_receipt.txn_id = "batch-before-reader".into();
        let mut probe_inboxes = inboxes[..64].to_vec();
        probe_inboxes[0].json_data = serde_json::json!({"barrier": true});
        let writer = tokio::spawn(async move {
            commit_to_device_transaction(&probe_receipt, &probe_inboxes, &[]).await
        });
        wait_for_lock(17, "ExclusiveLock").await;
        let snapshot = tokio::spawn(async move {
            crate::user::device::curr_sn_after_inbox_writes(&probe_user, &probe_device).await
        });
        wait_for_lock(0, "ShareLock").await;
        assert!(!snapshot.is_finished());
        diesel::sql_query("COMMIT")
            .execute(&mut controller)
            .await
            .unwrap();
        assert!(writer.await.unwrap().unwrap());
        assert_eq!(
            snapshot.await.unwrap().unwrap(),
            crate::curr_sn().await.unwrap()
        );

        let mut probe_receipt = second_receipt;
        probe_receipt.txn_id = "competing-large-retry".into();
        let probe_inboxes = inboxes[..64].to_vec();
        let before = counts().await;
        let (left, right) = tokio::join!(
            commit_to_device_transaction(&probe_receipt, &probe_inboxes, &[]),
            commit_to_device_transaction(&probe_receipt, &probe_inboxes, &[]),
        );
        assert_ne!(left.unwrap(), right.unwrap());
        assert_eq!(counts().await, (before.0 + 64, before.1, before.2 + 1));
    }
}
