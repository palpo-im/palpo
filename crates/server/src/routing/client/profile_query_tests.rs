use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedUserId, UnixMillis, UserId};
use crate::{config, data};

const URL: &str = "http://localhost/_matrix/client/unstable/org.matrix.msc4536/profile/query";

fn run(enabled: bool, rate: bool) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(query_http(enabled, rate));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_batch_profiles_known_local_data_and_validation() {
    run(true, false);
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_batch_profiles_disabled() {
    run(false, false);
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_batch_profiles_rate_limit() {
    run(true, true);
}

async fn request(service: &Service, request: RequestBuilder) -> (StatusCode, Value) {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        (
            response.status_code.unwrap(),
            response.take_json().await.unwrap_or(Value::Null),
        )
    })
    .await
    .unwrap()
}

async fn post(service: &Service, body: Value) -> (StatusCode, Value) {
    request(
        service,
        TestClient::post(URL)
            .add_header("Authorization", "Bearer profile-a", true)
            .json(&body),
    )
    .await
}

async fn user(id: &str, token: &str, local: bool) -> OwnedUserId {
    let user = UserId::parse(id).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: local,
        localpart: user.localpart().into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    if local {
        data::user::device::create_device(&user, &OwnedDeviceId::from("D"), token, None, None)
            .await
            .unwrap();
    }
    user
}

async fn query_http(enabled: bool, rate: bool) {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"profiles.example", "db":{"url":"unused"},
                "enable_batch_profile_queries":enabled,
                "rc_message":{"per_second":if rate { 0.000001 } else { 0.0 },"burst":1},
                "require_auth_for_profile_requests":true
            }))
            .unwrap(),
        )
        .unwrap();
    let alice = user("@alice:profiles.example", "profile-a", true).await;
    let bob = user("@bob:profiles.example", "profile-b", true).await;
    data::user::create_profile(&data::user::NewDbProfile {
        user_id: alice.clone(),
        room_id: None,
        display_name: None,
        avatar_url: Some("mxc://profiles.example/alice".try_into().unwrap()),
        blurhash: None,
    })
    .await
    .unwrap();
    data::user::set_display_name(&alice, "Alice").await.unwrap();
    data::user::set_profile_field(
        &alice,
        "org.example.status",
        json!({"emoji":"💬","text":"busy"}),
    )
    .await
    .unwrap();
    data::user::set_profile_field(&alice, "org.example.vector", json!([1, true, null]))
        .await
        .unwrap();
    let service = Service::new(crate::routing::root());
    let body = json!({"users":[alice]});
    for token in [None, Some("invalid")] {
        let req = TestClient::post(URL).json(&body);
        let req = if let Some(token) = token {
            req.add_header("Authorization", format!("Bearer {token}"), true)
        } else {
            req
        };
        assert_eq!(request(&service, req).await.0, StatusCode::UNAUTHORIZED);
    }
    let (status, first) = post(&service, body.clone()).await;
    assert_eq!(
        status,
        if enabled {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        },
        "{first}"
    );
    let (_, versions) = request(
        &service,
        TestClient::get("http://localhost/_matrix/client/versions"),
    )
    .await;
    assert_eq!(
        versions["unstable_features"]["org.matrix.msc4536"]
            .as_bool()
            .unwrap_or(false),
        enabled
    );
    if !enabled {
        return;
    }
    if rate {
        assert_eq!(
            post(&service, body.clone()).await.0,
            StatusCode::TOO_MANY_REQUESTS
        );
        let (status, _) = request(
            &service,
            TestClient::post(URL)
                .add_header("Authorization", "Bearer profile-b", true)
                .json(&body),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        return;
    }
    assert_eq!(
        first[alice.as_str()],
        json!({"displayname":"Alice","avatar_url":"mxc://profiles.example/alice", "org.example.status":{"emoji":"💬","text":"busy"},"org.example.vector":[1,true,null]})
    );
    assert!(first.get("profiles").is_none());

    // A second homeserver must receive no request, even if a remote cache exists.
    let peer = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_id = format!("@remote:{}", peer.local_addr().unwrap());
    let connections = Arc::new(AtomicUsize::new(0));
    let observed = connections.clone();
    let peer_task = tokio::spawn(async move {
        while let Ok((stream, _)) = peer.accept().await {
            observed.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    let remote = user(&remote_id, "unused", false).await;
    data::user::ensure_profile_exists(&remote).await.unwrap();
    data::user::set_display_name(&remote, "Remote cache must not be queried")
        .await
        .unwrap();
    let deactivated = user("@gone:profiles.example", "gone", true).await;
    data::user::ensure_profile_exists(&deactivated)
        .await
        .unwrap();
    data::user::mark_deactivated(&deactivated).await.unwrap();
    let empty = user("@empty:profiles.example", "empty", true).await;
    data::user::create_profile(&data::user::NewDbProfile {
        user_id: empty.clone(),
        room_id: None,
        display_name: None,
        avatar_url: None,
        blurhash: None,
    })
    .await
    .unwrap();
    // Room-only profile data must stay out of the global batch response.
    data::user::create_profile(&data::user::NewDbProfile {
        user_id: bob.clone(),
        room_id: Some("!private:profiles.example".try_into().unwrap()),
        display_name: Some("private room name".into()),
        avatar_url: None,
        blurhash: None,
    })
    .await
    .unwrap();
    let (status, mixed) = post(
        &service,
        json!({"users":[alice,alice,bob,empty,deactivated,remote,"@unknown:profiles.example"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{mixed}");
    assert_eq!(mixed, first);
    let (status, fields) = post(&service, json!({"users":[alice],"fields":["displayname","org.example.status","avatar_url","org.example.status"]})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        fields[alice.as_str()],
        json!({"displayname":"Alice","avatar_url":"mxc://profiles.example/alice","org.example.status":{"emoji":"💬","text":"busy"}})
    );
    assert_eq!(
        post(&service, json!({"users":[alice],"fields":["unknown"]}))
            .await
            .1,
        json!({alice.as_str():{}})
    );
    assert_eq!(
        post(
            &service,
            json!({"users":[remote,"@missing:profiles.example"]})
        )
        .await
        .1,
        json!({})
    );
    assert_eq!(
        post(&service, json!({"users":vec![alice.clone();100]}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        post(
            &service,
            json!({"users":[alice],"fields":vec!["displayname";30]})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        post(
            &service,
            json!({"users":[alice],"fields":["x".repeat(255)]})
        )
        .await
        .0,
        StatusCode::OK
    );
    for invalid in [
        json!(null),
        json!([]),
        json!({"users":null}),
        json!({"users":[]}),
        json!({"users":["invalid"]}),
        json!({"users":[true]}),
        json!({"users":vec![alice.clone();101]}),
        json!({"users":[alice],"fields":[]}),
        json!({"users":[alice],"fields":null}),
        json!({"users":[alice],"fields":"displayname"}),
        json!({"users":[alice],"fields":[1]}),
        json!({"users":[alice],"fields":[""]}),
        json!({"users":[alice],"fields":vec!["displayname";31]}),
        json!({"users":[alice],"fields":["界".repeat(86)]}),
    ] {
        let (status, error) = post(&service, invalid).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
        assert_eq!(error["errcode"], "M_BAD_JSON");
    }
    let (status, error) = post(&service, json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["errcode"], "M_MISSING_PARAM");
    // A storage outage is an error, rather than an apparently valid empty result.
    use diesel::sql_types::{Jsonb, Text};
    use horme::RunQueryDsl;
    let original_fields = data::user::profile_fields(&alice).await.unwrap();
    diesel::sql_query("UPDATE user_profiles SET fields=$2 WHERE user_id=$1 AND room_id IS NULL")
        .bind::<Text, _>(alice.as_str())
        .bind::<Jsonb, _>(json!({"org.example.legacy": "x".repeat(8 * 1024 * 1024)}))
        .execute(&mut data::db())
        .await
        .unwrap();
    let oversized = post(&service, body.clone()).await;
    diesel::sql_query("UPDATE user_profiles SET fields=$2 WHERE user_id=$1 AND room_id IS NULL")
        .bind::<Text, _>(alice.as_str())
        .bind::<Jsonb, _>(json!(original_fields))
        .execute(&mut data::db())
        .await
        .unwrap();
    assert_eq!(
        oversized.0,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        oversized.1
    );
    assert_eq!(oversized.1["errcode"], "M_TOO_LARGE");
    diesel::sql_query("ALTER TABLE user_profiles RENAME TO review_missing_profiles")
        .execute(&mut data::db())
        .await
        .unwrap();
    let failed = post(&service, body.clone()).await;
    diesel::sql_query("ALTER TABLE review_missing_profiles RENAME TO user_profiles")
        .execute(&mut data::db())
        .await
        .unwrap();
    assert_eq!(failed.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(post(&service, body).await.1, first);
    tokio::task::yield_now().await;
    assert_eq!(
        connections.load(Ordering::SeqCst),
        0,
        "batch query contacted a remote homeserver"
    );
    peer_task.abort();
}
