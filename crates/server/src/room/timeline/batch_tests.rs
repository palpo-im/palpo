use diesel::prelude::*;
use diesel::sql_types::{BigInt, Json, Text};
use horme::RunQueryDsl;
use serde_json::json;

use super::batch::{CHUNK_SIZE, PduBatch};
use super::topolo;
use crate::core::serde::JsonValue;
use crate::core::{OwnedEventId, OwnedRoomId, UnixMillis};
use crate::data::connect;
use crate::data::schema::{event_datas, rooms};
use crate::test_database::sql_spy::SqlSpy;

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_timeline_batch_preserves_events_and_bounds_storage_queries() {
    crate::test_database::init();
    crate::config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name": "timeline-perf.test", "db": {"url": "unused"},
                "auto_join_rooms": [], "ip_range_denylist": []
            }))
            .unwrap(),
        )
        .unwrap();
    let room: OwnedRoomId = "!batch:timeline-perf.test".try_into().unwrap();
    diesel::insert_into(rooms::table)
        .values(crate::data::room::NewDbRoom {
            id: room.clone(),
            version: "11".into(),
            is_public: false,
            min_depth: 0,
            has_auth_chain_index: false,
            created_at: UnixMillis(1),
        })
        .execute(&mut connect())
        .await
        .unwrap();
    let mut ids = Vec::new();
    for i in 0..260_i64 {
        let id: OwnedEventId = format!("$batch-{i}:timeline-perf.test").try_into().unwrap();
        let body = json!({
            "sender": "@owner:timeline-perf.test", "type": "m.room.message",
            "origin_server_ts": 1_700_000_000_000_u64, "depth": i + 1,
            "prev_events": [], "auth_events": [],
            "content": {"msgtype": "m.text", "body": format!("message {i}")},
            "hashes": {"sha256": "fixture"}, "signatures": {},
            "unsigned": if i == 0 { json!({"transaction_id": "perf-txn"}) } else { json!({}) }
        });
        diesel::sql_query("WITH stored AS (
            INSERT INTO events (id,sn,ty,room_id,sender_id,depth,topological_ordering,
                stream_ordering,origin_server_ts,contains_url,is_outlier,soft_failed,rejection_reason)
            VALUES ($1,$2,'m.room.message',$3,'@owner:timeline-perf.test',1,$2,
                CASE WHEN $2 % 3 = 0 THEN -$2 ELSE $2 END,1700000000000,false,$2 % 5 = 0,
                $2 % 7 = 0,CASE WHEN $2 % 11 = 0 THEN 'fixture rejection' END)
            ) INSERT INTO event_datas(event_id,event_sn,room_id,json_data) VALUES ($1,$2,$3,$4)")
            .bind::<Text,_>(id.as_str()).bind::<BigInt,_>(i + 1)
            .bind::<Text,_>(room.as_str()).bind::<Json,_>(body)
            .execute(&mut connect()).await.unwrap();
        ids.push(id);
    }
    diesel::update(event_datas::table.find(&ids[0]))
        .set(event_datas::internal_metadata.eq(json!({
            "transaction_user": "@owner:timeline-perf.test", "transaction_id": "perf-txn",
            "transaction_device": "PHONE"
        })))
        .execute(&mut connect())
        .await
        .unwrap();
    let spy = SqlSpy::install().await;
    spy.reset();
    for id in &ids[..20] {
        let _ = super::get_pdu(id).await.unwrap();
    }
    let storage_reads = || {
        spy.snapshot()
            .iter()
            .filter(|sql| {
                sql.contains("INNER JOIN")
                    && sql.contains("event_datas")
                    && sql.contains("\"events\"")
            })
            .count()
    };
    let single_reads = storage_reads();
    assert_eq!(single_reads, 20);
    spy.reset();
    let page = topolo::load_pdus_backward(None, &room, None, None, None, 20)
        .await
        .unwrap();
    let batch_reads = storage_reads();
    assert_eq!(batch_reads, 1);
    let expected_page = ids
        .iter()
        .enumerate()
        .rev()
        .filter(|(i, _)| (i + 1) % 5 != 0 && (i + 1) % 7 != 0)
        .take(20)
        .map(|(_, id)| id);
    assert!(page.values().map(|pdu| &pdu.event_id).eq(expected_page));
    println!("timeline page storage reads: single={single_reads}, batch={batch_reads}");
    ids.reverse();
    spy.reset();
    let mut batches = Vec::new();
    for chunk in ids.chunks(CHUNK_SIZE) {
        batches.push(PduBatch::load(chunk.iter().map(|id| &**id)).await.unwrap());
    }
    assert_eq!(
        storage_reads(),
        2,
        "260 unique events need two bounded reads"
    );
    spy.assert_coverage().await;
    for (i, id) in ids.iter().enumerate() {
        let expected = super::get_pdu(id).await.unwrap();
        let actual = batches[i / CHUNK_SIZE].take(id).await.unwrap();
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(actual.event_sn, expected.event_sn);
        assert_eq!(actual.is_outlier, expected.is_outlier);
        assert_eq!(actual.soft_failed, expected.soft_failed);
        assert_eq!(actual.is_backfill, expected.is_backfill);
        assert_eq!(actual.rejection_reason, expected.rejection_reason);
        assert_eq!(actual.transaction_device, expected.transaction_device);
        assert_eq!(actual.thread_participants, expected.thread_participants);
        if id == ids.last().unwrap() {
            assert_eq!(
                actual.transaction_device.as_deref().map(|id| id.as_str()),
                Some("PHONE")
            );
            for (device, visible) in [("PHONE", true), ("LAPTOP", false)] {
                let response: JsonValue = serde_json::from_str(
                    actual
                        .to_room_event_for(&actual.sender, Some(device.into()))
                        .as_str(),
                )
                .unwrap();
                assert_eq!(
                    response.pointer("/unsigned/transaction_id").is_some(),
                    visible
                );
            }
        }
    }
    ids.reverse();
    #[cfg(feature = "unstable-msc1763")]
    {
        // Expiry committed after prefetch must scrub the already-fetched body.
        let mut stale = PduBatch::load(std::iter::once(&*ids[3])).await.unwrap();
        diesel::sql_query("INSERT INTO event_retention_expired(event_id,expired_at) VALUES ($1,1)")
            .bind::<Text, _>(ids[3].as_str())
            .execute(&mut connect())
            .await
            .unwrap();
        let expected = super::get_pdu(&ids[3]).await.unwrap();
        let actual = stale.take(&ids[3]).await.unwrap();
        assert_eq!(
            serde_json::from_str::<JsonValue>(actual.content.get()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
    }
    // Prefetch does not eagerly decode later malformed JSON. Missing bodies and
    // unknown IDs retain the same not-found classification as the single loader.
    diesel::update(event_datas::table.find(&ids[1]))
        .set(event_datas::json_data.eq(json!({})))
        .execute(&mut connect())
        .await
        .unwrap();
    diesel::delete(event_datas::table.find(&ids[2]))
        .execute(&mut connect())
        .await
        .unwrap();
    let missing: OwnedEventId = "$absent:timeline-perf.test".try_into().unwrap();
    let mut batch = PduBatch::load([&*ids[0], &*ids[1], &*ids[2], &*missing, &*ids[0]].into_iter())
        .await
        .unwrap();
    assert!(batch.take(&ids[0]).await.is_ok());
    assert!(batch.take(&ids[1]).await.is_err());
    assert!(batch.take(&ids[2]).await.unwrap_err().is_not_found());
    assert!(batch.take(&missing).await.unwrap_err().is_not_found());
    spy.reset();
    let _empty = PduBatch::load(std::iter::empty()).await.unwrap();
    assert!(!spy.snapshot().iter().any(|sql| sql.contains("event_datas")));
    spy.uninstall().await;
}
