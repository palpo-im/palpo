//! Fixed-size history measurements and a logical snapshot round trip.
//!
//! Fixtures establish a trusted source explicitly. This is not a network/TLS
//! trust test or a production backup command; the notary suite covers transport.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use diesel::QueryableByName;
use diesel::sql_types::{Array, BigInt, Bool, Json, Jsonb, Text};
use horme::RunQueryDsl;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;
use crate::core::federation::discovery::OldVerifyKey;
use crate::core::signatures::Ed25519KeyPair;
use crate::test_database::sql_spy::SqlSpy;

const SERVER: &str = "history-capacity.example";
const SAMPLES: usize = 32;
const SIZES: [usize; 2] = [500, 2_000];

// These explain fixtures must match the SQL observed from the production
// selector, checked below. Bind values remain structured Diesel parameters.
const LATEST: &str = "SELECT d.key_data,d.document_sha256,d.source,d.source_server,d.validation_version,d.validated_at FROM verified_server_key_documents d WHERE d.server_id=$1 AND d.validation_version=1 AND d.validated_at>=0 AND ((d.source='direct' AND d.source_server=d.server_id) OR (d.source='notary' AND d.source_server=ANY($2))) ORDER BY d.validated_at DESC,d.id DESC LIMIT 1";
const EVENT: &str = "SELECT d.key_data,d.document_sha256,d.source,d.source_server,d.validation_version,d.validated_at FROM verified_server_key_versions v JOIN verified_server_key_documents d ON d.id=v.document_id AND d.server_id=v.server_id WHERE v.server_id=$1 AND v.key_id=$3 AND d.validation_version=1 AND d.validated_at>=0 AND ((d.source='direct' AND d.source_server=d.server_id) OR (d.source='notary' AND d.source_server=ANY($2))) AND (NOT $5 OR ((CASE WHEN v.retired THEN v.valid_until>$4 ELSE v.valid_until>=$4 END) AND d.validated_at::numeric+604800000 >= $4::numeric)) ORDER BY d.validated_at DESC,d.id DESC LIMIT 8";
const RETIREMENT: &str = "SELECT min(LEAST(v.valid_until::numeric,d.validated_at::numeric+604800000))::bigint AS expiry FROM verified_server_key_versions v JOIN verified_server_key_documents d ON d.id=v.document_id AND d.server_id=v.server_id WHERE v.server_id=$1 AND v.key_id=$3 AND v.retired AND d.validation_version=1 AND d.validated_at>=0 AND ((d.source='direct' AND d.source_server=d.server_id) OR (d.source='notary' AND d.source_server=ANY($2)))";

fn pair(version: &str) -> Ed25519KeyPair {
    Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), version.into()).unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn event(pair: &Ed25519KeyPair, timestamp: u64) -> CanonicalJsonObject {
    let mut event = serde_json::from_value(json!({
        "type":"m.room.message","sender":format!("@alice:{SERVER}"),
        "room_id":format!("!history:{SERVER}"),"origin_server_ts":timestamp,
        "content":{"msgtype":"m.text","body":"fixed history capacity payload"},
        "auth_events":[],"prev_events":[],"depth":1
    }))
    .unwrap();
    signatures::hash_and_sign_event(SERVER, pair, &mut event, &RoomVersionRules::V5.redaction)
        .unwrap();
    event
}

async fn seed(
    index: usize,
    now: u64,
    active: &Ed25519KeyPair,
    ancient: &Ed25519KeyPair,
    retired: &Ed25519KeyPair,
) {
    let server: &ServerName = SERVER.try_into().unwrap();
    let mut keys = ServerSigningKeys::new(server.to_owned(), UnixMillis(now + 86_400_000));
    let public = VerifyKey::from_bytes(active.public_key().to_vec());
    keys.verify_keys
        .insert("ed25519:active".try_into().unwrap(), public.clone());
    keys.verify_keys.insert(
        format!("ed25519:history_{index:04}").try_into().unwrap(),
        public,
    );
    if index == 0 {
        keys.verify_keys.insert(
            "ed25519:ancient".try_into().unwrap(),
            VerifyKey::from_bytes(ancient.public_key().to_vec()),
        );
        keys.verify_keys.insert(
            "ed25519:retired".try_into().unwrap(),
            VerifyKey::from_bytes(retired.public_key().to_vec()),
        );
    }
    if index == 499 || index == 1_999 {
        keys.old_verify_keys.insert(
            "ed25519:retired".try_into().unwrap(),
            OldVerifyKey::new(
                UnixMillis(now - 1_000),
                Base64::new(retired.public_key().to_vec()),
            ),
        );
    }
    let mut raw = serde_json::to_value(keys).unwrap();
    raw["org.palpo.capacity_fixture"] = json!({"sequence":index,"padding":"x".repeat(128)});
    let mut canonical: CanonicalJsonObject = serde_json::from_value(raw).unwrap();
    signatures::sign_json(SERVER, active, &mut canonical).unwrap();
    let raw = serde_json::to_value(canonical).unwrap();
    let verified = trust::origin(&raw, server).unwrap();
    trust::persist(&raw, &verified, "direct", server)
        .await
        .unwrap();
}

#[derive(Clone, Copy)]
enum Lookup {
    Http,
    Ancient,
    Common,
    RetiredHttp,
    RetiredEvent,
}

impl Lookup {
    fn name(self) -> &'static str {
        match self {
            Self::Http => "http-current",
            Self::Ancient => "event-ancient",
            Self::Common => "event-common",
            Self::RetiredHttp => "http-retired",
            Self::RetiredEvent => "event-before-retirement",
        }
    }

    fn loaded(self) -> usize {
        match self {
            Self::Http | Self::Ancient => 1,
            Self::Common => 8,
            Self::RetiredHttp => 0,
            Self::RetiredEvent => 3,
        }
    }
}

async fn lookup(
    case: Lookup,
    ancient: &CanonicalJsonObject,
    common: &CanonicalJsonObject,
    retired: &CanonicalJsonObject,
) {
    let server: &ServerName = SERVER.try_into().unwrap();
    match case {
        Lookup::Http => {
            get_verify_key(server, "ed25519:active".try_into().unwrap())
                .await
                .unwrap();
        }
        Lookup::Ancient => {
            verify_event(ancient, &RoomVersionId::V5).await.unwrap();
        }
        Lookup::Common => {
            verify_event(common, &RoomVersionId::V5).await.unwrap();
        }
        Lookup::RetiredHttp => {
            assert!(
                get_verify_key(server, "ed25519:retired".try_into().unwrap())
                    .await
                    .is_err()
            );
        }
        Lookup::RetiredEvent => {
            verify_event(retired, &RoomVersionId::V5).await.unwrap();
        }
    }
}

fn sql_queries(spy: &SqlSpy) -> Vec<String> {
    spy.snapshot()
        .into_iter()
        .filter_map(|query| {
            let sql = query.split("-- binds").next().unwrap().trim();
            sql.contains("verified_server_key_").then(|| sql.to_owned())
        })
        .collect()
}

fn percentiles(times: &mut [Duration]) -> Value {
    times.sort();
    let millis =
        |percent: usize| times[(times.len() * percent).div_ceil(100) - 1].as_secs_f64() * 1_000.0;
    json!({"p50_ms":millis(50),"p95_ms":millis(95),"p99_ms":millis(99)})
}

#[derive(QueryableByName)]
struct Plan {
    #[diesel(sql_type = Json, column_name = "QUERY PLAN")]
    value: Value,
}

async fn plans(observed: &[String], now: u64, size: usize) {
    for sql in [LATEST, EVENT, RETIREMENT] {
        assert!(
            observed.iter().any(|actual| actual == sql),
            "explain fixture must match the live production SQL: {sql}"
        );
    }
    let server: &ServerName = SERVER.try_into().unwrap();
    let notaries: Vec<String> = Vec::new();
    let latest = diesel::sql_query(format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {LATEST}"))
        .bind::<Text, _>(server.as_str())
        .bind::<Array<Text>, _>(&notaries)
        .get_result::<Plan>(&mut data::connect())
        .await
        .unwrap();
    for (case, id) in [
        ("event-ancient", "ed25519:ancient"),
        ("event-common", "ed25519:active"),
    ] {
        let plan = diesel::sql_query(format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {EVENT}"))
            .bind::<Text, _>(server.as_str())
            .bind::<Array<Text>, _>(&notaries)
            .bind::<Text, _>(id)
            .bind::<BigInt, _>(now as i64)
            .bind::<Bool, _>(true)
            .get_result::<Plan>(&mut data::connect())
            .await
            .unwrap();
        println!(
            "T06_HISTORY_PLAN {}",
            json!({"documents":size,"case":case,"plan":plan.value})
        );
    }
    let retired = diesel::sql_query(format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {RETIREMENT}"
    ))
    .bind::<Text, _>(server.as_str())
    .bind::<Array<Text>, _>(&notaries)
    .bind::<Text, _>("ed25519:retired")
    .get_result::<Plan>(&mut data::connect())
    .await
    .unwrap();
    println!(
        "T06_HISTORY_PLAN {}",
        json!({"documents":size,"case":"http-current","plan":latest.value})
    );
    println!(
        "T06_HISTORY_PLAN {}",
        json!({"documents":size,"case":"retirement","plan":retired.value})
    );
}

const SNAPSHOT: &str = "SELECT jsonb_build_object(
    'documents',(SELECT COALESCE(jsonb_agg(to_jsonb(d) ORDER BY d.id),'[]'::jsonb) FROM verified_server_key_documents d WHERE server_id=$1),
    'versions',(SELECT COALESCE(jsonb_agg(to_jsonb(v) ORDER BY v.document_id,v.key_id,v.retired),'[]'::jsonb) FROM verified_server_key_versions v WHERE server_id=$1)
) AS snapshot";

#[derive(QueryableByName)]
struct Snapshot {
    #[diesel(sql_type = Jsonb)]
    snapshot: Value,
}

#[derive(QueryableByName)]
struct Storage {
    #[diesel(sql_type = BigInt)]
    document_table_bytes: i64,
    #[diesel(sql_type = BigInt)]
    document_index_bytes: i64,
    #[diesel(sql_type = BigInt)]
    version_table_bytes: i64,
    #[diesel(sql_type = BigInt)]
    version_index_bytes: i64,
}

async fn storage(size: usize) {
    let value = diesel::sql_query("SELECT pg_table_size('verified_server_key_documents')::bigint AS document_table_bytes, pg_indexes_size('verified_server_key_documents')::bigint AS document_index_bytes, pg_table_size('verified_server_key_versions')::bigint AS version_table_bytes, pg_indexes_size('verified_server_key_versions')::bigint AS version_index_bytes")
        .get_result::<Storage>(&mut data::connect()).await.unwrap();
    println!(
        "T06_HISTORY_STORAGE {}",
        json!({"documents":size,"document_table_bytes":value.document_table_bytes,"document_index_bytes":value.document_index_bytes,"version_table_bytes":value.version_table_bytes,"version_index_bytes":value.version_index_bytes,"scope":"whole relations in dedicated fixture database, including PostgreSQL page overhead"})
    );
}

async fn snapshot() -> Value {
    diesel::sql_query(SNAPSHOT)
        .bind::<Text, _>(SERVER)
        .get_result::<Snapshot>(&mut data::connect())
        .await
        .unwrap()
        .snapshot
}

async fn restore(archive: &Value) -> AppResult<()> {
    let mut connection = data::connect();
    connection.transaction::<_,crate::AppError,_>(async |conn| {
        diesel::sql_query("DELETE FROM verified_server_key_documents WHERE server_id=$1")
            .bind::<Text,_>(SERVER).execute(conn).await?;
        diesel::sql_query("INSERT INTO verified_server_key_documents SELECT * FROM jsonb_populate_recordset(NULL::verified_server_key_documents,$1)")
            .bind::<Jsonb,_>(&archive["documents"]).execute(conn).await?;
        diesel::sql_query("INSERT INTO verified_server_key_versions SELECT * FROM jsonb_populate_recordset(NULL::verified_server_key_versions,$1)")
            .bind::<Jsonb,_>(&archive["versions"]).execute(conn).await?;
        Ok(())
    }).await
}

#[tokio::test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
async fn review_verified_history_capacity_and_snapshot_restore() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"history-review-local.example","db":{"url":"unused"},
                "trusted_servers":[],"only_query_trusted_key_servers":true,
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
            }))
            .unwrap(),
        )
        .unwrap();
    let now = UnixMillis::now().0;
    let active = pair("active");
    let ancient = pair("ancient");
    let retired = pair("retired");
    let ancient_event = event(&ancient, now);
    let common_event = event(&active, now);
    let retired_event = event(&retired, now - 2_000);
    let rejected_event = event(&retired, now);
    let cases = [
        Lookup::Http,
        Lookup::Ancient,
        Lookup::Common,
        Lookup::RetiredHttp,
        Lookup::RetiredEvent,
    ];
    let loaded = || AtomicUsize::load(&trust::RAW_DOCUMENTS_LOADED, Ordering::Relaxed);
    let mut prior_size = 0;
    let mut expected_queries = None;
    for size in SIZES {
        for index in prior_size..size {
            seed(index, now, &active, &ancient, &retired).await;
        }
        for table in [
            "verified_server_key_documents",
            "verified_server_key_versions",
        ] {
            diesel::sql_query(format!("ANALYZE {table}"))
                .execute(&mut data::connect())
                .await
                .unwrap();
        }
        let spy = SqlSpy::install().await;
        let mut timings: [Vec<Duration>; 5] = std::array::from_fn(|_| Vec::new());
        let mut counts: [Vec<usize>; 5] = std::array::from_fn(|_| Vec::new());
        let mut observed = Vec::new();
        for sample in 0..SAMPLES {
            // Alternate case order so the same path does not always warm the
            // relation cache for every following path.
            for offset in 0..cases.len() {
                let index = if sample.is_multiple_of(2) {
                    offset
                } else {
                    cases.len() - 1 - offset
                };
                spy.reset();
                let before = loaded();
                let started = Instant::now();
                lookup(cases[index], &ancient_event, &common_event, &retired_event).await;
                timings[index].push(started.elapsed());
                let queries = sql_queries(&spy);
                counts[index].push(queries.len());
                observed.extend(queries);
                let expected_loaded = match cases[index] {
                    Lookup::RetiredEvent => {
                        if size == 500 {
                            2
                        } else {
                            3
                        }
                    }
                    other => other.loaded(),
                };
                assert_eq!(
                    loaded() - before,
                    expected_loaded,
                    "{} loaded bound at {size}",
                    cases[index].name()
                );
            }
        }
        let actual_queries: Vec<_> = counts
            .iter()
            .map(|counts| {
                assert!(counts.windows(2).all(|w| w[0] == w[1]));
                counts[0]
            })
            .collect();
        if let Some(expected) = &expected_queries {
            assert_eq!(
                &actual_queries, expected,
                "quadrupling history must not add selector round trips"
            );
        }
        expected_queries = Some(actual_queries);
        for (index, case) in cases.iter().enumerate() {
            println!(
                "T06_HISTORY_MEASUREMENT {}",
                json!({
                    "documents":size,"case":case.name(),"samples":SAMPLES,"os":std::env::consts::OS,
                    "arch":std::env::consts::ARCH,"anchor_ms":now,"query_pool_size":data::status().max_size,
                    "sql_per_lookup":counts[index],"latency":percentiles(&mut timings[index]),
                    "scope":"same-process, warmed local PostgreSQL; not production capacity or an SLO"
                })
            );
        }
        spy.assert_coverage().await;
        spy.uninstall().await;
        plans(&observed, now, size).await;
        storage(size).await;
        prior_size = size;
    }
    assert!(
        verify_event(&rejected_event, &RoomVersionId::V5)
            .await
            .is_err()
    );
    let archive = snapshot().await;
    assert_eq!(archive["documents"].as_array().unwrap().len(), 2_000);
    assert_eq!(archive["versions"].as_array().unwrap().len(), 4_004);
    let encoded = serde_json::to_vec(&archive).unwrap();
    let digest = sha256(&encoded);
    // JSON serialization models an external logical archive. Preserve every
    // provenance/expiry/observation field; calling persist here would re-date it.
    let decoded: Value = serde_json::from_slice(&encoded).unwrap();
    let mut invalid = decoded.clone();
    invalid["versions"][0]["document_id"] = json!(-1);
    assert!(
        restore(&invalid).await.is_err(),
        "a broken document/index archive must fail its foreign key check"
    );
    assert_eq!(
        snapshot().await,
        archive,
        "an incomplete restore must roll back without destroying existing history"
    );
    restore(&decoded).await.unwrap();
    assert_eq!(
        snapshot().await,
        archive,
        "restoration must preserve raw bodies, selector indexes, trust and observation times exactly"
    );
    for case in cases {
        lookup(case, &ancient_event, &common_event, &retired_event).await;
    }
    assert!(
        verify_event(&rejected_event, &RoomVersionId::V5)
            .await
            .is_err()
    );
    println!(
        "T06_HISTORY_RESTORE {}",
        json!({"documents":2_000,"versions":4_004,"archive_bytes":encoded.len(),"archive_sha256":digest,"raw_and_metadata_exact":true,"current_http":true,"historical_events":true,"retirement_preserved":true,"scope":"dedicated-database logical snapshot, no PostgreSQL service or storage failure"})
    );
}
