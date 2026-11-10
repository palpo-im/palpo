use serde_json::json;

use super::*;
use crate::core::federation::discovery::OldVerifyKey;
use crate::core::room_version_rules::RoomVersionRules;
use crate::core::serde::CanonicalJsonObject;
use crate::core::signatures::Ed25519KeyPair;

fn pair() -> Ed25519KeyPair {
    Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), "test".into()).unwrap()
}

fn document(pair: &Ed25519KeyPair, until: u64) -> ServerSigningKeys {
    let mut keys = ServerSigningKeys::new("remote.example".try_into().unwrap(), UnixMillis(until));
    keys.verify_keys.insert(
        format!("ed25519:{}", pair.version()).try_into().unwrap(),
        VerifyKey::from_bytes(pair.public_key().to_vec()),
    );
    let mut raw: CanonicalJsonObject =
        serde_json::from_value(serde_json::to_value(&keys).unwrap()).unwrap();
    signatures::sign_json("remote.example", pair, &mut raw).unwrap();
    serde_json::from_value(serde_json::to_value(raw).unwrap()).unwrap()
}

fn event(pair: &Ed25519KeyPair, timestamp: u64, rules: &RoomVersionRules) -> CanonicalJsonObject {
    event_for("remote.example", pair, timestamp, rules)
}

fn event_for(
    origin: &str,
    pair: &Ed25519KeyPair,
    timestamp: u64,
    rules: &RoomVersionRules,
) -> CanonicalJsonObject {
    let mut object = serde_json::from_value(json!({
        "type":"m.room.message", "sender":format!("@alice:{origin}"), "room_id":format!("!room:{origin}"),
        "content":{"msgtype":"m.text","body":"signed boundary"}, "auth_events":[], "prev_events":[],
        "depth":1, "origin_server_ts":timestamp
    })).unwrap();
    signatures::hash_and_sign_event(origin, pair, &mut object, &rules.redaction).unwrap();
    object
}

#[test]
fn event_key_validity_boundaries_and_legacy_room_versions() {
    let pair = pair();
    let mut keys = document(&pair, 100);
    let id: &ServerSigningKeyId = "ed25519:test".try_into().unwrap();
    assert!(select_key(&keys, id, UnixMillis(100), true).is_some());
    assert!(select_key(&keys, id, UnixMillis(101), true).is_none());
    assert!(select_key(&keys, id, UnixMillis(101), false).is_some());
    let old = keys.verify_keys.remove(id).unwrap();
    keys.old_verify_keys
        .insert(id.to_owned(), OldVerifyKey::new(UnixMillis(80), old.key));
    assert!(select_key(&keys, id, UnixMillis(79), true).is_some());
    assert!(select_key(&keys, id, UnixMillis(80), true).is_none());
    assert!(select_key(&keys, id, UnixMillis(101), false).is_some());
}

#[test]
fn cached_key_horizon_is_anchored_to_fetch_time() {
    let pair = pair();
    // Signed documents must stay within canonical JSON's integer range.
    let mut keys = document(&pair, (1_u64 << 53) - 1);
    bound_cached_keys(&mut keys, UnixMillis(10));
    assert_eq!(keys.valid_until_ts, UnixMillis(10 + MAX_KEY_LIFETIME_MS));
    assert!(
        select_key(
            &keys,
            "ed25519:test".try_into().unwrap(),
            UnixMillis(11 + MAX_KEY_LIFETIME_MS),
            true
        )
        .is_none()
    );
}

#[test]
fn rotated_keys_do_not_inherit_the_replacement_validity() {
    let pair = pair();
    let keys = document(&pair, 100);
    let replacement = ServerSigningKeys::new("remote.example".try_into().unwrap(), UnixMillis(900));
    let merged = merge_signing_keys_for_storage(Some(keys), replacement);
    assert!(
        select_key(
            &merged,
            "ed25519:test".try_into().unwrap(),
            UnixMillis(200),
            true
        )
        .is_none()
    );
    assert!(
        select_key(
            &merged,
            "ed25519:test".try_into().unwrap(),
            UnixMillis(99),
            true
        )
        .is_some()
    );
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_event_verification_enforces_cached_key_expiry() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                    "server_name":"local.example", "db":{"url":"unused"},
                    "trusted_servers":[], "only_query_trusted_key_servers":true,
            "federation":{"allowed_servers":["local.example"]}
                }))
            .unwrap(),
        )
        .unwrap();
    let pair = pair();
    let now = UnixMillis::now().0;
    let remote: &ServerName = "remote.example".try_into().unwrap();
    let keys = document(&pair, now);
    trust::seed(keys.clone(), &pair).await;
    let historical = event(&pair, now - 1, &RoomVersionRules::V5);
    assert!(
        super::super::verify_event(&historical, &RoomVersionId::V5)
            .await
            .is_ok()
    );
    let expired = event(&pair, now + 1, &RoomVersionRules::V5);
    assert!(
        super::super::verify_event(&expired, &RoomVersionId::V5)
            .await
            .is_err()
    );
    // v4 deliberately ignores validity; this also exercises the actual async
    // server verifier rather than only testing the filtering helper.
    let legacy = event(&pair, now + 1, &RoomVersionRules::V4);
    assert!(
        super::super::verify_event(&legacy, &RoomVersionId::V4)
            .await
            .is_ok()
    );

    // Renewal of a still-current key can extend its event lifetime.
    trust::seed(document(&pair, now + 60_000), &pair).await;
    assert!(
        super::super::verify_event(&expired, &RoomVersionId::V5)
            .await
            .is_ok()
    );
    let replacement =
        Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), "replacement".into())
            .unwrap();
    let mut retired = keys.clone();
    let key = retired.verify_keys.remove("ed25519:test").unwrap();
    retired.old_verify_keys.insert(
        "ed25519:test".try_into().unwrap(),
        OldVerifyKey::new(UnixMillis(now - 1), key.key),
    );
    trust::seed(retired, &replacement).await;
    assert!(
        super::super::verify_event(&historical, &RoomVersionId::V5)
            .await
            .is_err()
    );
    let older = event(&pair, now - 2, &RoomVersionRules::V5);
    assert!(
        super::super::verify_event(&older, &RoomVersionId::V5)
            .await
            .is_ok()
    );

    // Replaying a previously valid current document cannot undo retirement.
    trust::seed(document(&pair, now + 60_000), &pair).await;
    assert!(
        super::super::verify_event(&expired, &RoomVersionId::V5)
            .await
            .is_err()
    );
    let stale =
        Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), "stale".into()).unwrap();
    let mut rotated = document(&replacement, now + 60_000);
    rotated.old_verify_keys.insert(
        "ed25519:stale".try_into().unwrap(),
        OldVerifyKey::new(
            UnixMillis(now - 1),
            Base64::new(stale.public_key().to_vec()),
        ),
    );
    trust::seed(rotated, &replacement).await;
    let mut double_signed = expired.clone();
    signatures::hash_and_sign_event(
        "remote.example",
        &replacement,
        &mut double_signed,
        &RoomVersionRules::V5.redaction,
    )
    .unwrap();
    signatures::hash_and_sign_event(
        "remote.example",
        &stale,
        &mut double_signed,
        &RoomVersionRules::V5.redaction,
    )
    .unwrap();
    assert!(
        super::super::verify_event(&double_signed, &RoomVersionId::V5)
            .await
            .is_ok()
    );
    let signatures = double_signed
        .get_mut("signatures")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("remote.example")
        .unwrap()
        .as_object_mut()
        .unwrap();
    signatures.insert("ed25519:unknown".into(), signatures["ed25519:test"].clone());
    assert!(
        super::super::verify_event(&double_signed, &RoomVersionId::V5)
            .await
            .is_ok()
    );
    assert!(
        data::misc::signing_keys_record(remote)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_local_retired_key_obeys_event_expiry_without_refresh() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"local-key.example", "db":{"url":"unused"},
                "trusted_servers":[], "federation":{"enable":false}
            }))
            .unwrap(),
        )
        .unwrap();
    let pair = pair();
    let now = UnixMillis::now().0;
    let mut keys = ServerSigningKeys::new(
        "local-key.example".try_into().unwrap(),
        UnixMillis(now + 60_000),
    );
    keys.old_verify_keys.insert(
        "ed25519:test".try_into().unwrap(),
        OldVerifyKey::new(UnixMillis(now), Base64::new(pair.public_key().to_vec())),
    );
    trust::seed(keys, config::keypair()).await;
    let before = event_for("local-key.example", &pair, now - 1, &RoomVersionRules::V5);
    assert!(
        super::super::verify_event(&before, &RoomVersionId::V5)
            .await
            .is_ok()
    );
    let after = event_for("local-key.example", &pair, now, &RoomVersionRules::V5);
    assert!(
        super::super::verify_event(&after, &RoomVersionId::V5)
            .await
            .is_err()
    );
    let current = event_for(
        "local-key.example",
        config::keypair(),
        now,
        &RoomVersionRules::V5,
    );
    assert!(
        super::super::verify_event(&current, &RoomVersionId::V5)
            .await
            .is_ok()
    );
    let legacy = event_for("local-key.example", &pair, now, &RoomVersionRules::V4);
    assert!(
        super::super::verify_event(&legacy, &RoomVersionId::V4)
            .await
            .is_ok()
    );
}

use crate::server_key::test_fixture as notary_fixture;

#[derive(Clone)]
struct NotaryFixture {
    bootstrap: serde_json::Value,
    response: std::sync::Arc<std::sync::Mutex<(salvo::http::StatusCode, serde_json::Value)>>,
    requests: std::sync::Arc<std::sync::Mutex<Vec<(String, Option<u64>)>>>,
}

#[async_trait::async_trait]
impl salvo::Handler for NotaryFixture {
    async fn handle(
        &self,
        req: &mut salvo::Request,
        _depot: &mut salvo::Depot,
        res: &mut salvo::Response,
        _ctrl: &mut salvo::FlowCtrl,
    ) {
        if req.uri().path() == "/_matrix/key/v2/server" {
            res.render(salvo::writing::Json(self.bootstrap.clone()));
            return;
        }
        self.requests.lock().unwrap().push((
            req.uri().path().to_string(),
            req.query::<u64>("minimum_valid_until_ts"),
        ));
        let (status, body) = self.response.lock().unwrap().clone();
        res.status_code(status);
        res.render(salvo::writing::Json(body));
    }
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_event_cache_refreshes_from_programmable_notary() {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use salvo::conn::Acceptor;
    use salvo::conn::rustls::{Keycert, RustlsConfig};
    use salvo::prelude::*;

    crate::test_database::init();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let acceptor = TcpListener::new("127.0.0.1:0")
        .rustls(RustlsConfig::new(
            Keycert::new()
                .cert(notary_fixture::CERT)
                .key(notary_fixture::KEY),
        ))
        .bind()
        .await;
    let address = acceptor.holdings()[0]
        .local_addr
        .clone()
        .into_std()
        .unwrap();
    let notary = format!("localhost:{}", address.port());
    let notary_pair = pair();
    let mut bootstrap = document(&notary_pair, UnixMillis::now().0 + 60_000);
    bootstrap.server_name = notary.as_str().try_into().unwrap();
    bootstrap.signatures.clear();
    let mut raw: CanonicalJsonObject =
        serde_json::from_value(serde_json::to_value(&bootstrap).unwrap()).unwrap();
    signatures::sign_json(&notary, &notary_pair, &mut raw).unwrap();
    let fixture = NotaryFixture {
        bootstrap: serde_json::to_value(raw).unwrap(),
        response: Arc::new(Mutex::new((StatusCode::OK, json!({"server_keys":[]})))),
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let server = tokio::spawn(
        Server::new(acceptor).serve(
            Router::new()
                .push(Router::with_path("_matrix/key/v2/query/{server_name}").get(fixture.clone()))
                .push(Router::with_path("_matrix/key/v2/server").get(fixture.clone())),
        ),
    );
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"notary-client.example", "db":{"url":"unused"},
                "trusted_servers":[format!("localhost:{}", address.port())],
                "query_trusted_key_servers_first":true, "only_query_trusted_key_servers":true,
                "ip_range_denylist":[], "allow_invalid_tls_certificates":true,
                "http_client":{"federation_retries":0}, "request_conn_timeout":1000,
                "request_timeout":1000,"request_total_timeout":2000
            }))
            .unwrap(),
        )
        .unwrap();
    let pair = pair();
    let now = UnixMillis::now().0;
    let event_time = now + 1_000;
    trust::seed(document(&pair, now - 1), &pair).await;
    let mut refreshed: CanonicalJsonObject =
        serde_json::from_value(serde_json::to_value(document(&pair, now + 60_000)).unwrap())
            .unwrap();
    signatures::sign_json(&notary, &notary_pair, &mut refreshed).unwrap();
    *fixture.response.lock().unwrap() = (StatusCode::OK, json!({"server_keys":[refreshed]}));
    let event = event(&pair, event_time, &RoomVersionRules::V5);
    let checked = tokio::time::timeout(
        Duration::from_secs(10),
        super::super::verify_event(&event, &RoomVersionId::V5),
    )
    .await
    .unwrap();
    assert!(checked.is_ok(), "{checked:?}");
    let first_requests = fixture.requests.lock().unwrap().clone();
    assert_eq!(first_requests.len(), 1);
    assert_eq!(first_requests[0].0, "/_matrix/key/v2/query/remote.example");
    assert!(first_requests[0].1.unwrap() >= event_time);
    // The refresh must persist its validity and serve the next check from cache.
    assert!(
        super::super::verify_event(&event, &RoomVersionId::V5)
            .await
            .is_ok()
    );
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    let cached = signing_keys_for("remote.example".try_into().unwrap())
        .await
        .unwrap();
    assert_eq!(cached.valid_until_ts, UnixMillis(now + 60_000));

    // A failed refresh cannot revive the expired cached key.
    // Use an event beyond every verified document, rather than making an
    // authenticated newer lifetime disappear by overwriting a cache row.
    let event = self::event(&pair, now + 120_000, &RoomVersionRules::V5);
    *fixture.response.lock().unwrap() = (
        StatusCode::SERVICE_UNAVAILABLE,
        json!({"errcode":"M_UNKNOWN", "error":"fixture unavailable"}),
    );
    let checked = tokio::time::timeout(
        Duration::from_secs(10),
        super::super::verify_event(&event, &RoomVersionId::V5),
    )
    .await
    .unwrap();
    assert!(checked.is_err());
    assert_eq!(fixture.requests.lock().unwrap().len(), 2);
    server.abort();
}
