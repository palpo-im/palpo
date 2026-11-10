//! Fixed-data PostgreSQL comparison of the old and batched federation loaders.
use std::time::{Duration, Instant};

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;
use crate::core::RoomVersionId;
use crate::core::serde::{Base64, CanonicalJsonValue};
use crate::core::signatures::{self, Ed25519KeyPair, PublicKeyMap};
use crate::test_database::sql_spy::SqlSpy;

const ORIGIN: &str = "pdu-batch.example";
const SAMPLES: usize = 50;

async fn legacy(server: &ServerName, ids: &[OwnedEventId]) -> AppResult<Vec<Box<RawJsonValue>>> {
    let mut result = Vec::new();
    for id in ids {
        let json = timeline::get_pdu_json(id)
            .await?
            .ok_or_else(|| AppError::internal(format!("event not found: {server} {id}")))?;
        result.push(convert_to_outgoing_federation_event(json).await);
    }
    Ok(result)
}

fn payloads(pdus: &[Box<RawJsonValue>]) -> Vec<String> {
    pdus.iter().map(|pdu| pdu.get().to_owned()).collect()
}

async fn room(version: &RoomVersionId) -> OwnedRoomId {
    let id: OwnedRoomId = format!("!v{version}:{ORIGIN}").try_into().unwrap();
    diesel::insert_into(rooms::table)
        .values(data::room::NewDbRoom {
            id: id.clone(),
            version: version.to_string(),
            is_public: false,
            min_depth: 0,
            has_auth_chain_index: false,
            created_at: UnixMillis::now(),
        })
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();
    id
}

async fn store(id: &EventId, room: &RoomId, body: &Value) {
    let sn = data::next_sn().await.unwrap();
    diesel::insert_into(events::table)
        .values((
            events::id.eq(id),
            events::sn.eq(sn),
            events::ty.eq("m.room.message"),
            events::room_id.eq(room),
            events::depth.eq(1),
            events::topological_ordering.eq(sn),
            events::stream_ordering.eq(sn),
            events::origin_server_ts.eq(UnixMillis::now()),
            events::contains_url.eq(false),
            events::is_outlier.eq(false),
        ))
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();
    data::room::DbEventData {
        event_id: id.to_owned(),
        event_sn: sn,
        room_id: room.to_owned(),
        internal_metadata: None,
        format_version: None,
        json_data: body.clone(),
    }
    .save()
    .await
    .unwrap();
}

fn signed(
    key: &Ed25519KeyPair,
    id: &EventId,
    room: &RoomId,
    version: &RoomVersionId,
    no_room: bool,
) -> Value {
    let mut body = json!({
        "type":"m.room.message","room_id":room,"sender":format!("@alice:{ORIGIN}"),
        "origin":ORIGIN,"origin_server_ts":UnixMillis::now(),"depth":1,
        "prev_events":[],"auth_events":[],"content":{"msgtype":"m.text","body":"fixed batch payload"},
        "unsigned":{"transaction_id":"private-txn","org.matrix.msc4140.delay_id":"private-delay",
            "age":42,"m.relations":{"m.thread":{"latest_event":{"unsigned":{"transaction_id":"nested-private","age":5}}}}}
    });
    if matches!(version, RoomVersionId::V1 | RoomVersionId::V2) {
        body["event_id"] = json!(id);
    }
    if no_room {
        body.as_object_mut().unwrap().remove("room_id");
    }
    let mut canonical = serde_json::from_value(body).unwrap();
    signatures::hash_and_sign_event(
        ORIGIN,
        key,
        &mut canonical,
        &crate::room::get_version_rules(version).unwrap().redaction,
    )
    .unwrap();
    canonical.insert(
        "event_id".into(),
        CanonicalJsonValue::String(id.to_string()),
    );
    canonical.insert("event_sn".into(), CanonicalJsonValue::Integer(123.into()));
    serde_json::to_value(canonical).unwrap()
}

fn data_queries(spy: &SqlSpy) -> usize {
    spy.snapshot()
        .iter()
        .filter(|query| {
            let sql = query.split("-- binds").next().unwrap();
            ["event_datas", "\"events\"", "\"rooms\"", "event_retention"]
                .iter()
                .any(|table| sql.contains(table))
        })
        .count()
}

fn percentiles(samples: &mut [Duration]) -> Value {
    samples.sort();
    let millis = |percent: usize| {
        samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)].as_secs_f64() * 1000.0
    };
    json!({"p50_ms":millis(50),"p95_ms":millis(95),"p99_ms":millis(99)})
}

async fn measure(server: &ServerName, ids: &[OwnedEventId], label: &str, spy: &SqlSpy) {
    let expected = payloads(&legacy(server, ids).await.unwrap());
    assert_eq!(
        payloads(&pdu_batch::prepare(server, ids).await.unwrap()),
        expected
    );
    let (mut old_times, mut new_times, mut old_queries, mut new_queries) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut old_total, mut new_total) = (Vec::new(), Vec::new());
    for sample in 0..SAMPLES {
        // Alternate execution order to avoid assigning all warm-cache effects
        // to one implementation. Both paths consume exactly the same rows.
        for old in if sample % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        } {
            spy.reset();
            let started = Instant::now();
            let result = if old {
                legacy(server, ids).await
            } else {
                pdu_batch::prepare(server, ids).await
            }
            .unwrap();
            let elapsed = started.elapsed();
            let queries = data_queries(spy);
            let total = spy.snapshot().len();
            assert_eq!(payloads(&result), expected);
            if old {
                old_times.push(elapsed);
                old_queries.push(queries);
                old_total.push(total);
            } else {
                new_times.push(elapsed);
                new_queries.push(queries);
                new_total.push(total);
            }
        }
    }
    assert!(old_queries.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(new_queries.windows(2).all(|pair| pair[0] == pair[1]));
    if ids.len() > 1 {
        assert!(
            new_queries[0] < old_queries[0],
            "{label}: {old_queries:?} -> {new_queries:?}"
        );
    }
    println!(
        "P03_MEASUREMENT {}",
        json!({
            "case":label,"samples":SAMPLES,"batch_size":ids.len(),
            "data_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&expected).unwrap())),
            "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
            "query_pool_size":data::status().max_size,"retention_feature":cfg!(feature="unstable-msc1763"),
            "legacy":{"data_queries":old_queries[0],"all_start_query_events":old_total,"latency":percentiles(&mut old_times)},
            "batched":{"data_queries":new_queries[0],"all_start_query_events":new_total,"latency":percentiles(&mut new_times)}
        })
    );
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_federation_pdu_batch_preserves_semantics_and_measures_queries() {
    std::thread::Builder::new()
        .name("pdu-batch-review".into())
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
    config::CONFIG
        .set(serde_json::from_value(json!({"server_name":ORIGIN,"db":{"url":"unused"}})).unwrap())
        .unwrap();
    let key =
        Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), "batch".into()).unwrap();
    let public: PublicKeyMap = [(
        ORIGIN.into(),
        [(
            "ed25519:batch".into(),
            Base64::new(key.public_key().to_vec()),
        )]
        .into(),
    )]
    .into();
    let server: &ServerName = "peer.example".try_into().unwrap();
    let versions = [
        RoomVersionId::V1,
        RoomVersionId::V2,
        RoomVersionId::V3,
        RoomVersionId::V11,
        RoomVersionId::V12,
    ];
    let mut fixtures = Vec::new();
    let mut per_room = Vec::new();
    for version in &versions {
        let room = room(version).await;
        let mut ids = Vec::new();
        for index in 0..50 {
            let id: OwnedEventId = format!("$batch-v{version}-{index}:{ORIGIN}")
                .try_into()
                .unwrap();
            let body = signed(&key, &id, &room, version, version == &RoomVersionId::V12);
            store(&id, &room, &body).await;
            if index == 0 {
                fixtures.push((id.clone(), room.clone(), version.clone(), body));
            }
            ids.push(id);
        }
        per_room.push(ids);
    }
    let semantic_ids = [
        fixtures[3].0.clone(),
        fixtures[0].0.clone(),
        fixtures[4].0.clone(),
        fixtures[0].0.clone(),
        fixtures[1].0.clone(),
        fixtures[2].0.clone(),
    ];
    let expected = legacy(server, &semantic_ids).await.unwrap();
    let actual = pdu_batch::prepare(server, &semantic_ids).await.unwrap();
    assert_eq!(payloads(&actual), payloads(&expected));
    for (raw, id) in actual.iter().zip(&semantic_ids) {
        let fixture = fixtures.iter().find(|fixture| &fixture.0 == id).unwrap();
        let body: Value = serde_json::from_str(raw.get()).unwrap();
        assert_eq!(body["signatures"], fixture.3["signatures"]);
        assert_eq!(body["hashes"], fixture.3["hashes"]);
        assert_eq!(
            body.get("event_id").is_some(),
            matches!(fixture.2, RoomVersionId::V1 | RoomVersionId::V2)
        );
        assert!(body.get("event_sn").is_none());
        assert!(body["unsigned"].get("transaction_id").is_none());
        assert!(
            body["unsigned"]
                .get("org.matrix.msc4140.delay_id")
                .is_none()
        );
        assert!(
            body["unsigned"]["m.relations"]["m.thread"]["latest_event"]["unsigned"]
                .get("transaction_id")
                .is_none()
        );
        let canonical = serde_json::from_value(body).unwrap();
        signatures::verify_event(
            &public,
            &canonical,
            &crate::room::get_version_rules(&fixture.2).unwrap(),
        )
        .unwrap();
    }
    // Preserve JSON-room precedence and JSON-event-ID fallback rather than
    // silently trusting the storage row's room or the queued event ID.
    let mut tricky = fixtures[4].3.clone();
    tricky["room_id"] = json!(fixtures[0].1);
    let tricky_id: OwnedEventId = format!("$tricky:{ORIGIN}").try_into().unwrap();
    store(&tricky_id, &fixtures[4].1, &tricky).await;
    let mut fallback = fixtures[4].3.clone();
    fallback["event_id"] = json!(fixtures[1].0);
    let fallback_id: OwnedEventId = format!("$fallback:{ORIGIN}").try_into().unwrap();
    store(&fallback_id, &fixtures[4].1, &fallback).await;
    let mut unknown = fixtures[3].3.clone();
    unknown["room_id"] = json!(format!("!not-stored:{ORIGIN}"));
    unknown["event_id"] = json!("$unknown");
    let unknown_id: OwnedEventId = "$orphan".try_into().unwrap();
    store(&unknown_id, &fixtures[3].1, &unknown).await;
    let edge = [tricky_id, fallback_id, unknown_id];
    assert_eq!(
        payloads(&pdu_batch::prepare(server, &edge).await.unwrap()),
        payloads(&legacy(server, &edge).await.unwrap())
    );
    let edge_body: Value =
        serde_json::from_str(pdu_batch::prepare(server, &edge).await.unwrap()[2].get()).unwrap();
    assert_eq!(edge_body["event_id"], "$unknown");

    let missing: OwnedEventId = "$missing".try_into().unwrap();
    let invalid: OwnedEventId = "$invalid".try_into().unwrap();
    store(&invalid, &fixtures[3].1, &json!([1])).await;
    for ids in [[missing.clone(), invalid.clone()], [invalid, missing]] {
        let old = legacy(server, &ids).await.unwrap_err().to_string();
        let new = pdu_batch::prepare(server, &ids)
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(new, old, "first error must follow input order");
    }
    #[cfg(feature = "unstable-msc1763")]
    {
        let expired: OwnedEventId = "$expired".try_into().unwrap();
        store(&expired, &fixtures[3].1, &fixtures[3].3).await;
        diesel::sql_query(
            "INSERT INTO event_retention_expired (event_id,expired_at) VALUES ($1,0)",
        )
        .bind::<diesel::sql_types::Text, _>(expired.as_str())
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();
        let ids = [expired.clone(), expired];
        let old = legacy(server, &ids).await.unwrap();
        let new = pdu_batch::prepare(server, &ids).await.unwrap();
        assert_eq!(payloads(&new), payloads(&old));
        assert_eq!(
            serde_json::from_str::<Value>(new[0].get()).unwrap()["content"],
            json!({})
        );
    }
    let spy = SqlSpy::install().await;
    spy.reset();
    assert!(pdu_batch::prepare(server, &[]).await.unwrap().is_empty());
    assert_eq!(data_queries(&spy), 0);
    // Parameter bounds apply to batches beyond today's normal send limit too.
    let mut large = (0..1_001)
        .map(|index| format!("$missing-{index}").try_into().unwrap())
        .collect::<Vec<OwnedEventId>>();
    large.push(fixtures[0].0.clone());
    spy.reset();
    let loaded = timeline::load_pdu_json_batch(&large).await.unwrap();
    assert_eq!(data_queries(&spy), 2);
    assert!(loaded.get(&fixtures[0].0).await.unwrap().is_some());
    measure(server, &per_room[3][..1], "one-pdu-v11", &spy).await;
    measure(server, &per_room[3], "50-pdus-one-v11-room", &spy).await;
    let mixed = (0..50)
        .map(|index| per_room[index % per_room.len()][index].clone())
        .collect::<Vec<_>>();
    measure(server, &mixed, "50-pdus-five-room-versions", &spy).await;
    measure(
        server,
        &per_room[4],
        "50-v12-pdus-without-json-room-id",
        &spy,
    )
    .await;
    spy.assert_coverage().await;
    spy.uninstall().await;
}
