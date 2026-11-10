//! Each case owns globals and an empty dedicated PostgreSQL database.
use std::sync::{Arc, Mutex};

use salvo::conn::Acceptor;
use salvo::conn::rustls::{Keycert, RustlsConfig};
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;
use crate::core::signatures::Ed25519KeyPair;
use crate::server_key::test_fixture as tls_fixture;

const LOCAL: &str = "trust-review.example";

fn pair(version: &str) -> Ed25519KeyPair {
    Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), version.into()).unwrap()
}

fn signed(mut raw: Value, server: &str, pair: &Ed25519KeyPair) -> Value {
    let mut object: CanonicalJsonObject = serde_json::from_value(raw.take()).unwrap();
    signatures::sign_json(server, pair, &mut object).unwrap();
    serde_json::to_value(object).unwrap()
}

fn document(server: &str, pair: &Ed25519KeyPair, until: u64) -> Value {
    let mut keys = ServerSigningKeys::new(server.try_into().unwrap(), UnixMillis(until));
    keys.verify_keys.insert(
        format!("ed25519:{}", pair.version()).try_into().unwrap(),
        VerifyKey::from_bytes(pair.public_key().to_vec()),
    );
    signed(serde_json::to_value(keys).unwrap(), server, pair)
}

#[derive(Clone)]
struct Peer {
    response: Arc<Mutex<Value>>,
    bootstrap: Arc<Mutex<Value>>,
    requests: Arc<Mutex<Vec<(String, String)>>>,
}

#[async_trait::async_trait]
impl Handler for Peer {
    async fn handle(
        &self,
        req: &mut Request,
        _depot: &mut Depot,
        res: &mut Response,
        _ctrl: &mut FlowCtrl,
    ) {
        self.requests
            .lock()
            .unwrap()
            .push((req.method().to_string(), req.uri().path().into()));
        let value = if req.uri().path() == "/_matrix/key/v2/server" {
            self.bootstrap.lock().unwrap().clone()
        } else {
            self.response.lock().unwrap().clone()
        };
        res.render(Json(value));
    }
}

struct RunningPeer(tokio::task::JoinHandle<()>);
impl Drop for RunningPeer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn peer() -> (String, Peer, RunningPeer) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let acceptor = TcpListener::new("127.0.0.1:0")
        .rustls(RustlsConfig::new(
            Keycert::new().cert(tls_fixture::CERT).key(tls_fixture::KEY),
        ))
        .bind()
        .await;
    let address = acceptor.holdings()[0]
        .local_addr
        .clone()
        .into_std()
        .unwrap();
    let peer = Peer {
        response: Arc::new(Mutex::new(json!({"server_keys":[]}))),
        bootstrap: Arc::new(Mutex::new(json!({}))),
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let server =
        tokio::spawn(Server::new(acceptor).serve(Router::with_path("{*path}").goal(peer.clone())));
    (
        format!("localhost:{}", address.port()),
        peer,
        RunningPeer(server),
    )
}

#[handler]
fn accepted(res: &mut Response) {
    res.render("authenticated");
}

async fn authenticate(service: &Service, origin: &str, pair: &Ed25519KeyPair) -> StatusCode {
    let object = signed(
        json!({"method":"GET","uri":"/probe","origin":origin,"destination":LOCAL}),
        origin,
        pair,
    );
    let id = format!("ed25519:{}", pair.version());
    let signature = object["signatures"][origin][&id].as_str().unwrap();
    let response=TestClient::get("http://localhost/probe").add_header("authorization",format!("X-Matrix origin=\"{origin}\",destination=\"{LOCAL}\",key=\"{id}\",sig=\"{signature}\""),true).send(service).await;
    response.status_code.unwrap()
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_notary_routes_verify_attestations_and_separate_request_from_event_keys() {
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
    let (notary_name, notary, _notary_server) = peer().await;
    let (origin_name, origin, _origin_server) = peer().await;
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":LOCAL,"db":{"url":"unused"},"trusted_servers":[&notary_name],
                "query_trusted_key_servers_first":true,"only_query_trusted_key_servers":true,
                "ip_range_denylist":[],"allow_invalid_tls_certificates":true,
                "http_client":{"federation_retries":0},"request_conn_timeout":1000,
                "request_timeout":1000,"request_total_timeout":2000
            }))
            .unwrap(),
        )
        .unwrap();
    let notary_id: &ServerName = notary_name.as_str().try_into().unwrap();
    let origin_id: &ServerName = origin_name.as_str().try_into().unwrap();
    let key_id: &ServerSigningKeyId = "ed25519:active".try_into().unwrap();
    let notary_key = pair("notary");
    let active = pair("active");
    let outsider = pair("active");
    let now = UnixMillis::now().0;
    let bootstrap = document(&notary_name, &notary_key, now + 60_000);
    *notary.bootstrap.lock().unwrap() = bootstrap.clone();
    let attest = |raw| signed(raw, &notary_name, &notary_key);

    // An upgrade must not silently promote the old, source-less merged cache.
    let legacy: &ServerName = "legacy.example".try_into().unwrap();
    data::misc::upsert_signing_keys(legacy, document(legacy.as_str(), &outsider, now + 60_000))
        .await
        .unwrap();
    assert!(get_verify_key(legacy, key_id).await.is_err());
    let mut legacy_event: CanonicalJsonObject=serde_json::from_value(json!({
        "type":"m.room.message","sender":"@alice:legacy.example","room_id":"!room:legacy.example",
        "content":{"msgtype":"m.text","body":"unproven cache"},"auth_events":[],"prev_events":[],"depth":1,"origin_server_ts":now
    })).unwrap();
    signatures::hash_and_sign_event(
        legacy.as_str(),
        &outsider,
        &mut legacy_event,
        &RoomVersionRules::V5.redaction,
    )
    .unwrap();
    assert!(
        verify_event(&legacy_event, &RoomVersionId::V5)
            .await
            .is_err()
    );
    assert!(
        data::misc::signing_keys_record(legacy)
            .await
            .unwrap()
            .is_some()
    );

    // Distinct TLS peers prove the POST really reaches the configured notary,
    // not the queried origin, even in trusted-only mode.
    let mut origin_doc = document(&origin_name, &active, now + 60_000);
    origin_doc["org.example.signed_extension"] = json!({"preserved_for_verification":true});
    origin_doc["verify_keys"]["ed25519:active"]["org.example.foo"] = json!({"nested_current":true});
    origin_doc["old_verify_keys"]["ed25519:historical"] = json!({
        "key":VerifyKey::from_bytes(active.public_key().to_vec()).key,"expired_ts":now-1_000,
        "org.example.foo":{"nested_old":true}
    });
    origin_doc = signed(origin_doc, &origin_name, &active);
    *notary.response.lock().unwrap() = json!({"server_keys":[attest(origin_doc),attest(document("outsider.example",&outsider,now+60_000))]});
    *origin.response.lock().unwrap() =
        json!({"server_keys":[attest(document("victim.example",&outsider,now+60_000))]});
    acquire_pubkeys(std::iter::once((origin_id, std::iter::once(key_id)))).await;
    assert!(origin.requests.lock().unwrap().is_empty());
    assert!(
        notary
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(method, path)| method == "POST" && path == "/_matrix/key/v2/query")
    );
    assert!(
        data::misc::signing_keys_record(origin_id)
            .await
            .unwrap()
            .is_some()
    );
    let preserved = signing_key_document_for(origin_id).await.unwrap();
    assert_eq!(
        preserved["org.example.signed_extension"]["preserved_for_verification"],
        true
    );
    trust::signature(
        &preserved,
        origin_id,
        &trust::current_keys(&trust::origin(&preserved, origin_id).unwrap()),
    )
    .unwrap();
    // Acting as a notary must publish the original signed body, including
    // extensions; never publish the legacy merged cache and stale signatures.
    let published_service = Service::new(crate::routing::root());
    let mut response = TestClient::get(format!(
        "http://localhost/_matrix/key/v2/query/{origin_name}"
    ))
    .send(&published_service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let published: Value = response.take_json().await.unwrap();
    let returned = &published["server_keys"][0];
    assert_eq!(
        returned["verify_keys"]["ed25519:active"]["org.example.foo"]["nested_current"],
        true
    );
    assert_eq!(
        returned["old_verify_keys"]["ed25519:historical"]["org.example.foo"]["nested_old"],
        true
    );
    trust::origin(returned, origin_id).unwrap();
    trust::signature(
        returned,
        config::get().server_name.as_ref(),
        &BTreeMap::from([(
            format!("ed25519:{}", config::keypair().version())
                .try_into()
                .unwrap(),
            VerifyKey::from_bytes(config::keypair().public_key().to_vec()),
        )]),
    )
    .unwrap();
    trust::signature(
        &preserved,
        notary_id,
        &trust::current_keys(&trust::origin(&bootstrap, notary_id).unwrap()),
    )
    .unwrap();
    assert!(
        data::misc::signing_keys_record("outsider.example".try_into().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        data::misc::signing_keys_record("victim.example".try_into().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    for server in ["outsider.example", "victim.example"] {
        assert!(
            data::misc::verified_key_documents(server.try_into().unwrap())
                .await
                .unwrap()
                .is_empty()
        );
    }
    let service = Service::new(
        Router::with_path("probe")
            .hoop(crate::hoops::auth_by_signatures)
            .get(accepted),
    );
    assert_eq!(
        authenticate(&service, &origin_name, &active).await,
        StatusCode::OK
    );

    // A GET response for another origin must neither satisfy the requested
    // same-named key ID nor populate the cache with an unrelated document.
    *notary.response.lock().unwrap() =
        json!({"server_keys":[attest(document("other.example",&outsider,now+60_000))]});
    assert_ne!(
        authenticate(&service, "missing.example", &outsider).await,
        StatusCode::OK
    );
    assert!(
        data::misc::signing_keys_record("other.example".try_into().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        data::misc::verified_key_documents("other.example".try_into().unwrap())
            .await
            .unwrap()
            .is_empty()
    );

    // Mixed responses: only valid origin signatures AND the configured
    // notary's attestation are accepted. Self-asserted notary keys do not count.
    let names = [
        "good.example",
        "bad-origin.example",
        "bad-notary.example",
        "tampered.example",
        "unsigned.example",
    ];
    let mut bad_origin = document(names[1], &active, now + 60_000);
    bad_origin["signatures"][names[1]]["ed25519:active"] = json!("invalid");
    let bad_notary = signed(
        document(names[2], &active, now + 60_000),
        &notary_name,
        &pair("notary"),
    );
    let mut tampered = attest(document(names[3], &active, now + 60_000));
    tampered["valid_until_ts"] = json!(now + 120_000);
    *notary.response.lock().unwrap() = json!({"server_keys":[attest(document(names[0],&active,now+60_000)),attest(bad_origin),bad_notary,tampered,document(names[4],&active,now+60_000)]});
    let requested: Vec<&ServerName> = names
        .iter()
        .map(|name| (*name).try_into().unwrap())
        .collect();
    acquire_pubkeys(
        requested
            .iter()
            .map(|server| (*server, std::iter::once(key_id))),
    )
    .await;
    for (index, server) in requested.iter().enumerate() {
        assert_eq!(
            data::misc::signing_keys_record(server)
                .await
                .unwrap()
                .is_some(),
            index == 0,
            "{server}"
        );
        assert_eq!(
            !data::misc::verified_key_documents(server)
                .await
                .unwrap()
                .is_empty(),
            index == 0,
            "raw cache {server}"
        );
    }
    assert_eq!(
        authenticate(&service, names[0], &active).await,
        StatusCode::OK
    );

    // Bootstrap is anchored at the configured HTTPS endpoint and must itself
    // have the expected identity, a valid current signature and current age.
    let fresh: &ServerName = "bootstrap-check.example".try_into().unwrap();
    *notary.response.lock().unwrap() =
        json!({"server_keys":[attest(document(fresh.as_str(),&active,now+60_000))]});
    for bad_bootstrap in [
        document("wrong-notary.example", &notary_key, now + 60_000),
        document(&notary_name, &notary_key, now - 1),
        {
            let mut value = bootstrap.clone();
            value["valid_until_ts"] = json!(now + 120_000);
            value
        },
    ] {
        *notary.bootstrap.lock().unwrap() = bad_bootstrap;
        assert!(
            request::notary_request_at(notary_id, fresh, UnixMillis(now))
                .await
                .is_err()
        );
        assert!(
            data::misc::signing_keys_record(fresh)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            data::misc::verified_key_documents(fresh)
                .await
                .unwrap()
                .is_empty()
        );
    }
    *notary.bootstrap.lock().unwrap() = bootstrap;
    assert_eq!(
        authenticate(&service, fresh.as_str(), &active).await,
        StatusCode::OK
    );

    // Rotation: current requests authenticate with the replacement, retired
    // keys cannot authenticate HTTP, but historical V5 events remain valid.
    let replacement = pair("replacement");
    let mut rotated =
        ServerSigningKeys::new(names[0].try_into().unwrap(), UnixMillis(now + 60_000));
    rotated.verify_keys.insert(
        "ed25519:replacement".try_into().unwrap(),
        VerifyKey::from_bytes(replacement.public_key().to_vec()),
    );
    rotated.old_verify_keys.insert(
        "ed25519:active".try_into().unwrap(),
        crate::core::federation::discovery::OldVerifyKey::new(
            UnixMillis(now - 1_000),
            Base64::new(active.public_key().to_vec()),
        ),
    );
    let rotated = attest(signed(
        serde_json::to_value(rotated).unwrap(),
        names[0],
        &replacement,
    ));
    *notary.response.lock().unwrap() = json!({"server_keys":[rotated]});
    assert_eq!(
        authenticate(&service, names[0], &replacement).await,
        StatusCode::OK
    );
    assert_ne!(
        authenticate(&service, names[0], &active).await,
        StatusCode::OK
    );
    let mut historical: CanonicalJsonObject=serde_json::from_value(json!({
        "type":"m.room.message","sender":format!("@alice:{}",names[0]),"room_id":format!("!room:{}",names[0]),
        "content":{"msgtype":"m.text","body":"historical"},"auth_events":[],"prev_events":[],"depth":1,"origin_server_ts":now-2_000
    })).unwrap();
    signatures::hash_and_sign_event(
        names[0],
        &active,
        &mut historical,
        &RoomVersionRules::V5.redaction,
    )
    .unwrap();
    assert!(verify_event(&historical, &RoomVersionId::V5).await.is_ok());
    // More history than the online document budget must not expand an HTTP
    // read or require loading newer documents to find an old event key.
    let bounded: &ServerName = "bounded-history.example".try_into().unwrap();
    let ancient = pair("ancient");
    let active_id = std::collections::BTreeSet::from([key_id.to_owned()]);
    let count = data::misc::EVENT_KEY_DOCUMENT_BUDGET as usize * 3 + 1;
    for index in 0..count {
        let mut raw = document(bounded.as_str(), &active, now + 60_000);
        raw["org.example.sequence"] = json!(index);
        if index == 0 {
            raw["verify_keys"]["ed25519:ancient"] =
                json!({"key":VerifyKey::from_bytes(ancient.public_key().to_vec()).key});
        }
        raw = signed(raw, bounded.as_str(), &active);
        *notary.response.lock().unwrap() = json!({"server_keys":[attest(raw)]});
        request::notary_request_for(notary_id, bounded, UnixMillis(now), Some(&active_id))
            .await
            .unwrap()
            .count();
    }
    assert_eq!(
        data::misc::verified_key_documents(bounded)
            .await
            .unwrap()
            .len(),
        count
    );
    let loaded = || {
        std::sync::atomic::AtomicUsize::load(
            &trust::RAW_DOCUMENTS_LOADED,
            std::sync::atomic::Ordering::Relaxed,
        )
    };
    let before = loaded();
    assert_eq!(
        authenticate(&service, bounded.as_str(), &active).await,
        StatusCode::OK
    );
    assert_eq!(
        loaded() - before,
        1,
        "HTTP must load exactly one latest raw document"
    );
    let make_event = |pair: &Ed25519KeyPair| {
        let mut event:CanonicalJsonObject=serde_json::from_value(json!({
            "type":"m.room.message","sender":format!("@alice:{bounded}"),"room_id":format!("!room:{bounded}"),
            "content":{"msgtype":"m.text","body":"bounded history"},"auth_events":[],"prev_events":[],"depth":1,"origin_server_ts":now
        })).unwrap();
        signatures::hash_and_sign_event(
            bounded.as_str(),
            pair,
            &mut event,
            &RoomVersionRules::V5.redaction,
        )
        .unwrap();
        event
    };
    let before = loaded();
    assert!(
        verify_event(&make_event(&ancient), &RoomVersionId::V5)
            .await
            .is_ok()
    );
    assert_eq!(
        loaded() - before,
        1,
        "SQL key filter must find an old key without loading unrelated history"
    );
    let before = loaded();
    assert!(
        verify_event(&make_event(&active), &RoomVersionId::V5)
            .await
            .is_ok()
    );
    assert_eq!(
        loaded() - before,
        data::misc::EVENT_KEY_DOCUMENT_BUDGET as usize
    );
    assert_ne!(
        authenticate(&service, bounded.as_str(), &ancient).await,
        StatusCode::OK,
        "HTTP may not fall back to latest-containing-id"
    );
    let unknown = pair("unknown");
    for index in 0..3 {
        let mut raw = document(bounded.as_str(), &active, now + 60_000);
        raw["org.example.irrelevant_nonce"] = json!(index);
        raw = signed(raw, bounded.as_str(), &active);
        *notary.response.lock().unwrap() = json!({"server_keys":[attest(raw)]});
        assert_ne!(
            authenticate(&service, bounded.as_str(), &unknown).await,
            StatusCode::OK
        );
    }
    assert_eq!(
        data::misc::verified_key_documents(bounded)
            .await
            .unwrap()
            .len(),
        count,
        "unknown-key failures must not append irrelevant document variants"
    );
    // A later document may omit historical keys; retain authenticated history.
    *notary.response.lock().unwrap() =
        json!({"server_keys":[attest(document(names[0],&replacement,now+120_000))]});
    request::notary_request_at(notary_id, names[0].try_into().unwrap(), UnixMillis(now))
        .await
        .unwrap()
        .count();
    assert!(verify_event(&historical, &RoomVersionId::V5).await.is_ok());
    // Replaying an older signed current document must not resurrect a retired
    // key, even if the replay is the latest observation in the raw cache.
    *notary.response.lock().unwrap() =
        json!({"server_keys":[attest(document(names[0],&active,now+60_000))]});
    request::notary_request_at(notary_id, names[0].try_into().unwrap(), UnixMillis(now))
        .await
        .unwrap()
        .count();
    assert_ne!(
        authenticate(&service, names[0], &active).await,
        StatusCode::OK
    );
    assert!(verify_event(&historical, &RoomVersionId::V5).await.is_ok());
    println!(
        "T03 two TLS peers: origin requests=0; notary POST and direct bootstrap observed; mixed cache accepted=1/rejected=4; current HTTP accepted, retired HTTP rejected, historical V5 accepted"
    );
}
