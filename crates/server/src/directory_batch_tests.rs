//! Fixed-data, real-PostgreSQL directory parity and query/latency evidence.
//!
//! The independent legacy oracle is never run against intentionally cyclic or
//! foreign state chains. Fixtures bypass publication/authentication deliberately;
//! the separate directory HTTP regression covers those public boundaries.
use std::sync::Arc;
use std::time::{Duration, Instant};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Json, Text};
use diesel_async::RunQueryDsl;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;
use crate::core::events::StateEventType;
use crate::core::{OwnedEventId, OwnedRoomId, RoomId, UnixMillis};
use crate::data::schema::*;
use crate::room::state::{self, CompressedEvent, CompressedState, StateDiff};
use crate::test_database::sql_spy::SqlSpy;
use crate::{config, data};

const ORIGIN: &str = "directory-batch.example";
const TYPES: [&str; 8] = [
    "m.room.create",
    "m.room.join_rules",
    "m.room.name",
    "m.room.topic",
    "m.room.canonical_alias",
    "m.room.history_visibility",
    "m.room.guest_access",
    "m.room.avatar",
];
const SAMPLES: usize = 24;
const CUSTOM_FIELDS: usize = 128;

struct Fixture {
    room: OwnedRoomId,
    frame: i64,
    full: CompressedState,
    create_id: OwnedEventId,
}

// All selected state has complete, parseable PDU envelopes and matching rows in
// all three event tables. Hash/signature strings are fixed trusted fixture data;
// this test does not pretend to exercise federation signature verification.
async fn event(room: &RoomId, kind: &str, key: &str, content: Value) -> (OwnedEventId, i64) {
    let sn = data::next_sn().await.unwrap();
    let id: OwnedEventId = format!("$directory-batch-{sn}:{ORIGIN}")
        .try_into()
        .unwrap();
    let sender = format!("@owner:{ORIGIN}");
    let body = json!({
        "event_id":id,"room_id":room,"type":kind,"state_key":key,"sender":sender,
        "origin":ORIGIN,"origin_server_ts":1_700_000_000_000_u64,"depth":1,
        "prev_events":[],"auth_events":[],"content":content,
        "hashes":{"sha256":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"},
        "signatures":{ORIGIN:{"ed25519:fixture":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}},
        "unsigned":{"age":42}
    });
    diesel::sql_query(
        "WITH stored_event AS (
        INSERT INTO events (id, sn, ty, room_id, sender_id, state_key, depth,
            topological_ordering, stream_ordering, origin_server_ts, contains_url, is_outlier)
        VALUES ($1, $2, $3, $4, $5, $6, 1, $2, $2, 1700000000000, false, false)
    ), stored_body AS (
        INSERT INTO event_datas (event_id, event_sn, room_id, json_data) VALUES ($1, $2, $4, $7)
    ) INSERT INTO event_points (event_id, event_sn, room_id) VALUES ($1, $2, $4)",
    )
    .bind::<Text, _>(id.as_str())
    .bind::<BigInt, _>(sn)
    .bind::<Text, _>(kind)
    .bind::<Text, _>(room.as_str())
    .bind::<Text, _>(&sender)
    .bind::<Text, _>(key)
    .bind::<Json, _>(&body)
    .execute(&mut data::connect().await.unwrap())
    .await
    .unwrap();
    (id, sn)
}

async fn frame(
    room: &RoomId,
    label: &str,
    parent: Option<i64>,
    appended: &CompressedState,
    disposed: &CompressedState,
) -> i64 {
    let id = state::ensure_frame(room, label.as_bytes().to_vec())
        .await
        .unwrap();
    state::save_state_delta(
        room,
        id,
        StateDiff {
            parent_id: parent,
            appended: Arc::new(appended.clone()),
            disposed: Arc::new(disposed.clone()),
        },
    )
    .await
    .unwrap();
    id
}

async fn point_room(room: &RoomId, frame: Option<i64>) {
    diesel::update(rooms::table.find(room))
        .set(rooms::state_frame_id.eq(frame))
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
}

async fn seed(index: usize, fields: &[i64], custom: &[i64]) -> Fixture {
    let room: OwnedRoomId = format!("!directory-{index:04}:{ORIGIN}")
        .try_into()
        .unwrap();
    diesel::insert_into(rooms::table)
        .values(data::room::NewDbRoom {
            id: room.clone(),
            version: "11".into(),
            is_public: true,
            min_depth: 0,
            has_auth_chain_index: false,
            created_at: UnixMillis(1_700_000_000_000),
        })
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    let mut create = json!({"room_version":"11"});
    match index % 3 {
        1 => create["type"] = json!("m.space"),
        2 => create["type"] = json!("org.example.custom"),
        _ => {}
    }
    if index % 17 == 16 {
        create["m.federate"] = json!("invalid boolean");
    }
    let join = match index % 6 {
        1 => json!({"join_rule":"knock"}),
        2 => json!({"join_rule":"knock_restricted","allow":[]}),
        3 => json!({"join_rule":"invite"}),
        4 => json!({"join_rule":42}),
        _ => json!({"join_rule":"public"}),
    };
    let contents = [
        create,
        join,
        if index % 7 == 6 {
            json!({"name":23})
        } else {
            json!({"name":format!("ÉCOLE 中文 directory-{index:04}")})
        },
        if index % 7 == 5 {
            json!({"topic":false})
        } else {
            json!({"topic":format!("Topic Straße ΩMEGA {index}")})
        },
        if index % 7 == 4 {
            json!({"alias":42})
        } else {
            json!({"alias":format!("#alias-{index:04}:{ORIGIN}"),"alt_aliases":[]})
        },
        if index % 7 == 3 {
            json!({"history_visibility":false})
        } else {
            json!({"history_visibility":if index % 2 == 0 {"world_readable"} else {"shared"}})
        },
        if index % 7 == 2 {
            json!({"guest_access":false})
        } else {
            json!({"guest_access":"can_join"})
        },
        if index % 7 == 0 {
            json!({"url":42})
        } else {
            json!({"url":format!("mxc://{ORIGIN}/avatar-{index}")})
        },
    ];
    let mut full = CompressedState::new();
    let mut create_id = None;
    for (kind, content) in contents.into_iter().enumerate() {
        // Absent optional events exercise display defaults, independently of
        // malformed-but-present typed content above.
        if index % 4 == 1 && kind >= 6 {
            continue;
        }
        let (id, sn) = event(&room, TYPES[kind], "", content).await;
        full.insert(CompressedEvent::new(fields[kind], sn));
        if kind == 0 {
            create_id = Some(id);
        }
    }
    // Unrelated state deliberately has no event/body mapping. Both paths must
    // leave it unresolved; a directory projection must not install this partial
    // view in the process-wide full-state cache.
    for (offset, field) in custom.iter().enumerate() {
        full.insert(CompressedEvent::new(
            *field,
            -1_000_000 - (index * CUSTOM_FIELDS + offset) as i64,
        ));
    }
    let base = frame(&room, "base", None, &full, &CompressedState::new()).await;
    let mut current = base;
    if index % 4 == 0 {
        let previous = *full
            .iter()
            .find(|entry| entry.field_id() == fields[2])
            .unwrap();
        let (_, sn) = event(
            &room,
            TYPES[2],
            "",
            json!({"name":format!("替换 ÉCOLE directory-{index:04}")}),
        )
        .await;
        let appended = [CompressedEvent::new(fields[2], sn)].into_iter().collect();
        let disposed = [previous].into_iter().collect();
        current = frame(&room, "renamed", Some(base), &appended, &disposed).await;
        full.remove(&previous);
        full.extend(appended);
    }
    point_room(&room, Some(current)).await;
    if index % 2 == 0 {
        // Negative stale counters are clamped identically by both paths.
        let count = if index % 14 == 0 {
            -2
        } else {
            (index % 5 + 2) as i64
        };
        diesel::insert_into(stats_room_currents::table)
            .values((
                stats_room_currents::room_id.eq(&room),
                stats_room_currents::joined_members.eq(count),
            ))
            .execute(&mut data::connect().await.unwrap())
            .await
            .unwrap();
    } else {
        // No stats row: actual joined memberships are counted. Include a leave
        // row so a fallback which counts all membership rows fails parity.
        for member in 0..(index % 5 + 3) {
            let user = format!("@member-{member}:{ORIGIN}");
            let membership = if member == 0 { "leave" } else { "join" };
            let (id, sn) = event(
                &room,
                "m.room.member",
                &user,
                json!({"membership":membership}),
            )
            .await;
            diesel::sql_query("INSERT INTO room_users (event_id, event_sn, room_id, user_id,
                user_server_id, sender_id, membership, created_at) VALUES ($1,$2,$3,$4,$5,$4,$6,1700000000000)")
                .bind::<Text, _>(id.as_str()).bind::<BigInt, _>(sn).bind::<Text, _>(room.as_str())
                .bind::<Text, _>(&user).bind::<Text, _>(ORIGIN).bind::<Text, _>(membership)
                .execute(&mut data::connect().await.unwrap()).await.unwrap();
        }
    }
    Fixture {
        room,
        frame: current,
        full,
        create_id: create_id.unwrap(),
    }
}

fn filter(value: Value) -> PublicRoomFilter {
    serde_json::from_value(value).unwrap()
}

async fn compare(
    limit: Option<usize>,
    since: Option<&str>,
    filter: &PublicRoomFilter,
    network: &RoomNetwork,
) -> Value {
    let old = get_local_public_rooms_legacy(limit, since, filter, network)
        .await
        .unwrap();
    let new = query::get_local_public_rooms(limit, since, filter, network)
        .await
        .unwrap();
    let expected = serde_json::to_value(old).unwrap();
    assert_eq!(
        serde_json::to_value(new).unwrap(),
        expected,
        "limit={limit:?} since={since:?} filter={filter:?} network={network:?}"
    );
    expected
}

async fn parity() {
    let all = compare(
        Some(0),
        None,
        &PublicRoomFilter::default(),
        &RoomNetwork::Matrix,
    )
    .await;
    assert!(!all["chunk"].as_array().unwrap().is_empty());
    for value in [
        json!({}),
        json!({"room_types":[]}),
        json!({"room_types":[null]}),
        json!({"room_types":["m.space"]}),
        json!({"room_types":["org.example.custom",null]}),
        json!({"room_types":["org.example.absent"]}),
        json!({"generic_search_term":"école"}),
        json!({"generic_search_term":"中文"}),
        json!({"generic_search_term":"替换"}),
        json!({"generic_search_term":"ωmega"}),
        json!({"generic_search_term":"alias-0002"}),
        json!({"generic_search_term":"straße","room_types":["org.example.custom"]}),
        json!({"generic_search_term":""}),
        json!({"generic_search_term":"absent search term"}),
    ] {
        let filter = filter(value);
        compare(Some(0), None, &filter, &RoomNetwork::Matrix).await;
        compare(Some(3), Some("n1"), &filter, &RoomNetwork::Matrix).await;
    }
    for network in [
        RoomNetwork::Matrix,
        RoomNetwork::All,
        RoomNetwork::ThirdParty("irc".into()),
    ] {
        for (limit, since) in [
            (None, None),
            (Some(1), None),
            (Some(10), None),
            (Some(3), Some("n10")),
            (Some(4), Some("p10")),
            (Some(0), Some("p10")),
            (Some(10), Some("n9999")),
            (Some(10), Some("p9999")),
        ] {
            compare(limit, since, &PublicRoomFilter::default(), &network).await;
        }
    }
    let filter = PublicRoomFilter::default();
    let mut next = None;
    let mut seen = Vec::new();
    loop {
        let page = compare(Some(7), next.as_deref(), &filter, &RoomNetwork::Matrix).await;
        seen.extend(page["chunk"].as_array().unwrap().iter().cloned());
        next = page["next_batch"].as_str().map(ToOwned::to_owned);
        if next.is_none() {
            break;
        }
    }
    assert_eq!(
        Value::Array(seen),
        all["chunk"],
        "forward pages are a partition"
    );
    let mut previous = format!("p{}", all["chunk"].as_array().unwrap().len());
    let mut reversed = Vec::new();
    loop {
        let page = compare(Some(7), Some(&previous), &filter, &RoomNetwork::Matrix).await;
        let mut entries = page["chunk"].as_array().unwrap().clone();
        entries.extend(reversed);
        reversed = entries;
        let Some(token) = page["prev_batch"].as_str() else {
            break;
        };
        previous = token.to_owned();
    }
    assert_eq!(
        Value::Array(reversed),
        all["chunk"],
        "backward pages are a partition"
    );
    for token in ["n", "p", "x1", "n-1", "n184467440737095516160", "é1"] {
        assert!(
            get_local_public_rooms_legacy(Some(3), Some(token), &filter, &RoomNetwork::Matrix)
                .await
                .is_err()
        );
        assert!(
            query::get_local_public_rooms(Some(3), Some(token), &filter, &RoomNetwork::Matrix)
                .await
                .is_err()
        );
    }
    compare(
        Some(1),
        Some(&format!("n{}", usize::MAX)),
        &filter,
        &RoomNetwork::Matrix,
    )
    .await;
    compare(
        Some(1),
        Some(&format!("p{}", usize::MAX)),
        &filter,
        &RoomNetwork::Matrix,
    )
    .await;
}

fn percentiles(times: &mut [Duration]) -> Value {
    times.sort();
    let value = |percent: usize| {
        times[(times.len() * percent).div_ceil(100).saturating_sub(1)].as_secs_f64() * 1000.0
    };
    json!({"p50_ms":value(50),"p95_ms":value(95),"p99_ms":value(99)})
}

fn data_queries(spy: &SqlSpy) -> usize {
    spy.snapshot()
        .iter()
        .filter(|query| {
            let sql = query.split("-- binds").next().unwrap();
            [
                "rooms",
                "room_state_",
                "event_points",
                "event_datas",
                "events",
                "room_users",
                "stats_room_currents",
                "event_retention",
            ]
            .iter()
            .any(|table| sql.contains(table))
        })
        .count()
}

async fn measure(size: usize, spy: &SqlSpy) {
    for (label, limit, filter) in [
        ("page1", Some(1), PublicRoomFilter::default()),
        ("page10", Some(10), PublicRoomFilter::default()),
        ("all", Some(0), PublicRoomFilter::default()),
        (
            "search",
            Some(10),
            filter(json!({"generic_search_term":"école"})),
        ),
    ] {
        let expected = compare(limit, None, &filter, &RoomNetwork::Matrix).await;
        let (
            mut old_times,
            mut new_times,
            mut old_queries,
            mut new_queries,
            mut old_total,
            mut new_total,
        ) = (
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        for sample in 0..SAMPLES {
            for old in if sample % 2 == 0 {
                [true, false]
            } else {
                [false, true]
            } {
                spy.reset();
                let start = Instant::now();
                let result = if old {
                    get_local_public_rooms_legacy(limit, None, &filter, &RoomNetwork::Matrix).await
                } else {
                    query::get_local_public_rooms(limit, None, &filter, &RoomNetwork::Matrix).await
                }
                .unwrap();
                let elapsed = start.elapsed();
                let queries = data_queries(spy);
                let total = spy.snapshot().len();
                assert_eq!(serde_json::to_value(result).unwrap(), expected);
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
        assert!(
            new_queries.iter().max().unwrap() < old_queries.iter().min().unwrap(),
            "{size}/{label}: directory SQL did not decrease: {old_queries:?} -> {new_queries:?}"
        );
        println!(
            "P04_MEASUREMENT {}",
            json!({
                "case":label,"public_candidates":size,"samples":SAMPLES,"cache":"legacy full-state cache warmed equally before sampling",
                "unrelated_state_fields_per_room":CUSTOM_FIELDS,"query_pool_size":data::status().max_size,
                "os":std::env::consts::OS,"arch":std::env::consts::ARCH,
                "retention_feature":cfg!(feature="unstable-msc1763"),
                "data_sha256":Sha256::digest(serde_json::to_vec(&expected).unwrap()).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
                "legacy":{"data_queries":old_queries,"all_start_query_events":old_total,"latency":percentiles(&mut old_times)},
                "batched":{"data_queries":new_queries,"all_start_query_events":new_total,"latency":percentiles(&mut new_times)}
            })
        );
    }
}

async fn faults(fields: &[i64], custom: &[i64]) {
    let filter = PublicRoomFilter::default();
    let expected = serde_json::to_value(
        query::get_local_public_rooms(Some(0), None, &filter, &RoomNetwork::Matrix)
            .await
            .unwrap(),
    )
    .unwrap();
    // Do not call the legacy oracle after adding these intentionally corrupt
    // chains: it was never designed to reject recursive/foreign ancestry.
    let fixtures = [
        seed(1001, fields, custom).await,
        seed(1004, fields, custom).await,
        seed(1007, fields, custom).await,
        seed(1008, fields, custom).await,
        seed(1013, fields, custom).await,
        seed(1014, fields, custom).await,
    ];
    point_room(&fixtures[0].room, None).await;
    // A cycle containing valid directory fields must still be rejected.
    let cyclic = frame(
        &fixtures[1].room,
        "cycle",
        Some(fixtures[1].frame),
        &CompressedState::new(),
        &CompressedState::new(),
    )
    .await;
    diesel::update(room_state_deltas::table.find(fixtures[1].frame))
        .set(room_state_deltas::parent_id.eq(cyclic))
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    // A current delta with a foreign parent is incomplete, even if its own
    // projected fields would otherwise suffice to produce a directory entry.
    diesel::update(room_state_deltas::table.find(fixtures[2].frame))
        .set(room_state_deltas::parent_id.eq(fixtures[0].frame))
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    let mut invalid: Vec<u8> = fixtures[3]
        .full
        .iter()
        .flat_map(|entry| entry.as_bytes().iter().copied())
        .collect();
    invalid.push(0);
    diesel::update(room_state_deltas::table.find(fixtures[3].frame))
        .set(room_state_deltas::appended.eq(invalid))
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    diesel::delete(event_datas::table.find(&fixtures[4].create_id))
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    // Frame ID points directly at another room's otherwise healthy state.
    point_room(&fixtures[5].room, Some(fixtures[0].frame)).await;
    let actual = tokio::time::timeout(
        Duration::from_secs(10),
        query::get_local_public_rooms(Some(0), None, &filter, &RoomNetwork::Matrix),
    )
    .await
    .expect("corrupt directory state must terminate")
    .unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        expected,
        "bad candidates must not leak or alter the filtered count"
    );
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_public_directory_batching_preserves_pages_and_measures_queries() {
    std::thread::Builder::new()
        .name("directory-batch-review".into())
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
    let mut fields = Vec::new();
    for kind in TYPES {
        fields.push(
            state::ensure_field_id(&StateEventType::from(kind), "")
                .await
                .unwrap(),
        );
    }
    let mut custom = Vec::new();
    for index in 0..CUSTOM_FIELDS {
        custom.push(
            state::ensure_field_id(
                &StateEventType::from("org.example.unrelated"),
                &format!("custom-{index}"),
            )
            .await
            .unwrap(),
        );
    }
    let mut fixtures = Vec::new();
    for index in 0..32 {
        fixtures.push(seed(index, &fields, &custom).await);
    }
    state::STATE_INFO_CACHE.lock().unwrap().clear();
    let spy = SqlSpy::install().await;
    let initial = query::get_local_public_rooms(
        Some(1),
        None,
        &PublicRoomFilter::default(),
        &RoomNetwork::Matrix,
    )
    .await
    .unwrap();
    assert_eq!(initial.chunk.len(), 1);
    assert_eq!(
        state::STATE_INFO_CACHE.lock().unwrap().len(),
        0,
        "directory projection must not cache a partial full state"
    );
    for fixture in &fixtures {
        let complete = state::load_frame_info(fixture.frame).await.unwrap();
        assert_eq!(*complete.last().unwrap().full_state, fixture.full);
    }
    // Prove fallback and counters have real rows and observable values rather
    // than making parity succeed accidentally on an empty directory.
    let all = compare(
        Some(0),
        None,
        &PublicRoomFilter::default(),
        &RoomNetwork::Matrix,
    )
    .await;
    assert!(
        all["chunk"]
            .as_array()
            .unwrap()
            .iter()
            .any(|room| room["num_joined_members"] == 0)
    );
    assert!(
        all["chunk"]
            .as_array()
            .unwrap()
            .iter()
            .any(|room| room["num_joined_members"] == 6)
    );
    parity().await;
    measure(32, &spy).await;
    for index in 32..96 {
        fixtures.push(seed(index, &fields, &custom).await);
    }
    parity().await;
    measure(96, &spy).await;
    spy.assert_coverage().await;
    faults(&fields, &custom).await;
    spy.assert_coverage().await;
    spy.uninstall().await;
}
