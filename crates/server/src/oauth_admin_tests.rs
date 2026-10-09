//! Run this test alone with an EMPTY, DEDICATED PALPO_TEST_DATABASE_URL.
use diesel_async::RunQueryDsl;
use palpo::core::{UnixMillis, UserId};
use palpo::{config, data};
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate as palpo;

const ADMIN_SCOPE: &str = "urn:matrix:client:cc.c10y.msc4484.server_administration";
const API_SCOPE: &str = "urn:matrix:client:api:*";

#[cfg(feature = "unstable-msc4363")]
static STALE_AUTH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn introspection(token: &str) -> Value {
    let mut response = json!({
        "active": true, "username": "admin", "device_id": "DEVICE",
        "scope": format!("{API_SCOPE} urn:matrix:client:device:DEVICE")
    });
    match token {
        "api-admin" => {}
        "api-ordinary" => response["username"] = json!("ordinary"),
        "admin-only" | "non-admin" => {
            response["scope"] = json!(format!("{ADMIN_SCOPE} urn:matrix:client:device:DEVICE"));
            if token == "non-admin" {
                response["username"] = json!("ordinary");
            }
        }
        "both" => {
            response["scope"] = json!(format!(
                "{ADMIN_SCOPE} {API_SCOPE} urn:matrix:client:device:DEVICE"
            ))
        }
        "unstable-api" => {
            response["scope"] = json!(
                "urn:matrix:org.matrix.msc2967.client:api:* urn:matrix:org.matrix.msc2967.client:device:DEVICE"
            )
        }
        "malformed" => {
            response["scope"] = json!(format!("{ADMIN_SCOPE}\turn:matrix:client:device:DEVICE"))
        }
        "conflicting" => {
            response["scope"] = json!(format!(
                "{ADMIN_SCOPE} urn:matrix:client:device:DEVICE urn:matrix:client:device:OTHER"
            ))
        }
        "mismatched" => response["device_id"] = json!("OTHER"),
        "unprovisioned" => response["username"] = json!("missing"),
        "no-username" => {
            response.as_object_mut().unwrap().remove("username");
        }
        #[cfg(feature = "unstable-msc4363")]
        "fresh" | "stale" | "future" | "missing-evidence" | "weak" | "expired"
        | "fresh-ordinary" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            response["scope"] = json!(format!(
                "{ADMIN_SCOPE} {API_SCOPE} urn:matrix:client:device:DEVICE"
            ));
            if token != "missing-evidence" {
                response["auth_time"] = json!(if token == "stale"
                    || std::sync::atomic::AtomicBool::load(
                        &STALE_AUTH,
                        std::sync::atomic::Ordering::SeqCst
                    ) {
                    now - 301
                } else if token == "future" {
                    now + 60
                } else {
                    now
                });
                response["acr"] = json!(if token == "weak" { "pwd" } else { "mfa" });
            }
            if token == "expired" {
                response["exp"] = json!(now - 1);
            }
            if token == "fresh-ordinary" {
                response["username"] = json!("ordinary");
            }
        }
        _ => response = json!({ "active": false }),
    }
    response
}

/// Small real HTTP introspection server, so tests exercise the complete token pipeline.
async fn mock_introspection() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let (body_start, content_length) = loop {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        assert!(
                            headers
                                .to_ascii_lowercase()
                                .contains("authorization: bearer fixture-secret")
                        );
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while bytes.len() < body_start + content_length {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                }
                let form =
                    url::form_urlencoded::parse(&bytes[body_start..body_start + content_length]);
                let token = form.into_owned().find(|(key, _)| key == "token").unwrap().1;
                let body = introspection(&token).to_string();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            });
        }
    });
    (format!("http://{address}/introspect"), task)
}

async fn provision(localpart: &str, admin: bool) {
    let id = UserId::parse(format!("@{localpart}:scope.example")).unwrap();
    diesel::insert_into(data::schema::users::table)
        .values(data::user::NewDbUser {
            id: id.clone(),
            ty: None,
            is_admin: admin,
            is_guest: false,
            is_local: true,
            localpart: localpart.to_owned(),
            server_name: id.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    diesel::insert_into(data::schema::user_devices::table)
        .values(data::user::NewDbUserDevice {
            user_id: id.clone(),
            device_id: "DEVICE".into(),
            display_name: None,
            user_agent: None,
            is_hidden: false,
            last_seen_ip: None,
            last_seen_at: None,
            created_at: UnixMillis::now(),
        })
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    diesel::insert_into(data::schema::user_access_tokens::table)
        .values(data::user::NewDbAccessToken::new(
            id,
            "DEVICE".into(),
            format!("native-{localpart}"),
            None,
        ))
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
}

#[handler]
async fn scope_context(depot: &mut Depot) -> Json<Value> {
    let info = depot.get_typed::<palpo::AuthedInfo>().unwrap();
    Json(json!({
        "delegated": info.is_delegated_auth(),
        "has_scopes": info.oauth_scopes.is_some(),
        "api": info.oauth_scopes.as_ref().is_some_and(|scopes| scopes.contains(API_SCOPE)),
        "admin": info.oauth_scopes.as_ref().is_some_and(|scopes| scopes.contains(ADMIN_SCOPE)),
    }))
}

async fn get(service: &Service, path: &str, token: &str) -> Response {
    TestClient::get(format!("http://localhost{path}"))
        .add_header("Authorization", format!("Bearer {token}"), true)
        .send(service)
        .await
}

async fn assert_error(mut response: Response, status: StatusCode, code: &str, challenge: bool) {
    assert_eq!(response.status_code, Some(status));
    #[cfg(feature = "unstable-msc4363")]
    if challenge {
        assert!(response.headers().get("WWW-Authenticate").is_none());
        let body = response.take_json::<Value>().await.unwrap();
        assert_eq!(
            body["errcode"],
            "org.matrix.msc4363.M_INSUFFICIENT_USER_AUTHENTICATION"
        );
        assert_eq!(
            body["org.matrix.msc4363.scope"],
            format!("{ADMIN_SCOPE} urn:matrix:client:device:DEVICE {API_SCOPE}")
        );
        return;
    }
    if challenge {
        assert_eq!(
            response
                .headers()
                .get("WWW-Authenticate")
                .unwrap()
                .to_str()
                .unwrap(),
            format!("Bearer error=\"insufficient_scope\", scope=\"{ADMIN_SCOPE}\"")
        );
    } else {
        assert!(response.headers().get("WWW-Authenticate").is_none());
    }
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["errcode"],
        code
    );
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
async fn oauth_admin_routes() {
    crate::test_database::init();
    let (endpoint, mock) = mock_introspection().await;
    config::CONFIG.set(serde_json::from_value(json!({
        "server_name": "scope.example", "db": {"url": "unused-test-config"},
        "ip_range_denylist": [], "admin": {"mas_secret": "fixture-secret"},
        "delegated_auth": {"enable": true, "introspection_endpoint": endpoint, "introspection_cache_ttl": 0}
    })).unwrap()).unwrap();
    provision("admin", true).await;
    provision("ordinary", false).await;
    provision("target", false).await;
    palpo::appservice::register_appservice(serde_json::from_value(json!({
        "id": "admin-scope-fixture", "url": null,
        "as_token": "appservice-admin", "hs_token": "fixture-hs-token",
        "sender_localpart": "admin",
        "namespaces": {"users": [{"exclusive": true, "regex": "^@admin:scope\\.example$"}], "aliases": [], "rooms": []}
    })).unwrap()).await.unwrap();
    let service = Service::new(
        Router::new()
            .push(
                Router::with_path("scope-context")
                    .hoop(palpo::hoops::auth_by_access_token)
                    .get(scope_context),
            )
            .push(palpo::routing::root()),
    );
    for (token, expected) in [
        (
            "both",
            json!({"delegated": true, "has_scopes": true, "api": true, "admin": true}),
        ),
        (
            "native-admin",
            json!({"delegated": false, "has_scopes": false, "api": false, "admin": false}),
        ),
        (
            "appservice-admin",
            json!({"delegated": false, "has_scopes": false, "api": false, "admin": false}),
        ),
    ] {
        let mut response = get(&service, "/scope-context", token).await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        assert_eq!(response.take_json::<Value>().await.unwrap(), expected);
    }
    let enabled = cfg!(feature = "unstable-msc4484");
    let mut routes = vec!["/_palpo/admin".to_owned(), "/_synapse/admin".to_owned()];
    for version in ["v1", "v3", "r0"] {
        for operation in ["whois", "lock", "suspend"] {
            routes.push(format!(
                "/_matrix/client/{version}/admin/{operation}/@target:scope.example"
            ));
        }
    }
    for operation in ["lock", "suspend"] {
        routes.push(format!(
            "/_matrix/client/unstable/uk.timedout.msc4323/admin/{operation}/@target:scope.example"
        ));
    }
    for path in &routes {
        let response = get(&service, path, "api-admin").await;
        if enabled {
            assert_error(response, StatusCode::UNAUTHORIZED, "M_FORBIDDEN", true).await;
        } else {
            assert_eq!(response.status_code, Some(StatusCode::OK), "{path}");
        }
        let response = get(&service, path, "admin-only").await;
        if enabled {
            assert_eq!(response.status_code, Some(StatusCode::OK), "{path}");
        } else {
            assert_error(response, StatusCode::UNAUTHORIZED, "M_UNKNOWN_TOKEN", false).await;
        }
        for token in ["both", "native-admin", "appservice-admin"] {
            assert_eq!(
                get(&service, path, token).await.status_code,
                Some(StatusCode::OK),
                "{path} {token}"
            );
        }
        let response = get(&service, path, "non-admin").await;
        assert_error(
            response,
            if enabled {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::UNAUTHORIZED
            },
            if enabled {
                "M_FORBIDDEN"
            } else {
                "M_UNKNOWN_TOKEN"
            },
            false,
        )
        .await;
        assert_error(
            get(&service, path, "native-ordinary").await,
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            false,
        )
        .await;
    }
    if enabled {
        let admin_id = UserId::parse("@admin:scope.example").unwrap();
        data::user::set_guest(&admin_id, true).await.unwrap();
        assert_error(
            get(&service, &routes[0], "api-admin").await,
            StatusCode::UNAUTHORIZED,
            "M_FORBIDDEN",
            true,
        )
        .await;
        assert!(data::user::get_user(&admin_id).await.unwrap().is_guest);
        data::user::set_guest(&admin_id, false).await.unwrap();
    }
    let whoami = "/_matrix/client/v3/account/whoami";
    for token in [
        "api-admin",
        "api-ordinary",
        "unstable-api",
        "both",
        "native-admin",
        "native-ordinary",
    ] {
        assert_eq!(
            get(&service, whoami, token).await.status_code,
            Some(StatusCode::OK)
        );
    }
    assert_error(
        get(&service, whoami, "admin-only").await,
        StatusCode::UNAUTHORIZED,
        "M_UNKNOWN_TOKEN",
        false,
    )
    .await;
    for token in [
        "inactive",
        "malformed",
        "conflicting",
        "mismatched",
        "unprovisioned",
        "no-username",
    ] {
        assert_error(
            get(&service, &routes[0], token).await,
            StatusCode::UNAUTHORIZED,
            "M_UNKNOWN_TOKEN",
            false,
        )
        .await;
    }
    // Legacy users retain self-whois; a scope cannot turn an ordinary OAuth user into an admin.
    let self_whois = "/_matrix/client/v3/admin/whois/@ordinary:scope.example";
    assert_eq!(
        get(&service, self_whois, "native-ordinary")
            .await
            .status_code,
        Some(StatusCode::OK)
    );
    if enabled {
        assert_error(
            get(&service, self_whois, "non-admin").await,
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            false,
        )
        .await;
    }
    let ordinary_self = get(&service, self_whois, "api-ordinary").await;
    if enabled {
        assert_error(ordinary_self, StatusCode::FORBIDDEN, "M_FORBIDDEN", false).await;
    } else {
        assert_eq!(ordinary_self.status_code, Some(StatusCode::OK));
    }
    let target = UserId::parse("@target:scope.example").unwrap();
    for path in routes
        .iter()
        .filter(|path| path.contains("/lock/") || path.contains("/suspend/"))
    {
        let lock = path.contains("/lock/");
        let field = if lock { "locked" } else { "suspended" };
        if enabled {
            let response = TestClient::put(format!("http://localhost{path}"))
                .add_header("Authorization", "Bearer api-admin", true)
                .json(&json!({field: true}))
                .send(&service)
                .await;
            assert_error(response, StatusCode::UNAUTHORIZED, "M_FORBIDDEN", true).await;
            let user = data::user::get_user(&target).await.unwrap();
            assert!(!user.is_locked() && !user.is_suspended());
        }
        for value in [true, false] {
            let response = TestClient::put(format!("http://localhost{path}"))
                .add_header("Authorization", "Bearer both", true)
                .json(&json!({field: value}))
                .send(&service)
                .await;
            assert_eq!(response.status_code, Some(StatusCode::OK), "{path}");
            let user = data::user::get_user(&target).await.unwrap();
            assert_eq!(
                if lock {
                    user.is_locked()
                } else {
                    user.is_suspended()
                },
                value
            );
        }
    }
    let mut versions = get(&service, "/_matrix/client/versions", "api-admin").await;
    let body = versions.take_json::<Value>().await.unwrap();
    assert_eq!(
        body["unstable_features"].get("org.continuwuity.msc4484.unstable"),
        enabled.then_some(&json!(true))
    );
    mock.abort();
}

#[cfg(feature = "unstable-msc4363")]
#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
async fn oauth_step_up_routes() {
    crate::test_database::init();
    let (endpoint, mock) = mock_introspection().await;
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"scope.example", "db":{"url":"unused-test-config"},
                "ip_range_denylist":[], "admin":{"mas_secret":"fixture-secret"},
                "delegated_auth":{"enable":true, "introspection_endpoint":endpoint,
                    "introspection_cache_ttl":3600, "admin_max_age":300, "admin_acr_values":"mfa"}
            }))
            .unwrap(),
        )
        .unwrap();
    provision("admin", true).await;
    provision("ordinary", false).await;
    provision("target", false).await;
    let service = Service::new(palpo::routing::root());
    let path = "/_matrix/client/v3/admin/lock/@target:scope.example";
    for token in ["stale", "future", "missing-evidence", "weak", "api-admin"] {
        let mut response = get(&service, path, token).await;
        assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
        let body = response.take_json::<Value>().await.unwrap();
        assert_eq!(
            body["errcode"],
            "org.matrix.msc4363.M_INSUFFICIENT_USER_AUTHENTICATION"
        );
        assert_eq!(body["org.matrix.msc4363.max_age"], 300);
        assert_eq!(body["org.matrix.msc4363.acr_values"], "mfa");
        assert_eq!(
            body["org.matrix.msc4363.scope"],
            format!("{ADMIN_SCOPE} urn:matrix:client:device:DEVICE {API_SCOPE}")
        );
    }
    // Ordinary API requests can cache evidence; administrative requests still fetch
    // current assurance and evaluate its original authentication timestamp.
    assert_eq!(
        get(&service, "/_matrix/client/v3/account/whoami", "fresh")
            .await
            .status_code,
        Some(StatusCode::OK)
    );
    assert_eq!(
        get(&service, path, "fresh").await.status_code,
        Some(StatusCode::OK)
    );
    STALE_AUTH.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_error(
        get(&service, path, "fresh").await,
        StatusCode::UNAUTHORIZED,
        "unused",
        true,
    )
    .await;
    STALE_AUTH.store(false, std::sync::atomic::Ordering::SeqCst);
    // Retrying with fresh authentication completes the protected state change.
    let response = TestClient::put(format!("http://localhost{path}"))
        .add_header("Authorization", "Bearer fresh", true)
        .json(&json!({"locked":true}))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let target = UserId::parse("@target:scope.example").unwrap();
    assert!(data::user::get_user(&target).await.unwrap().is_locked());
    assert_error(
        get(&service, path, "expired").await,
        StatusCode::UNAUTHORIZED,
        "M_UNKNOWN_TOKEN",
        false,
    )
    .await;
    assert_error(
        get(&service, path, "fresh-ordinary").await,
        StatusCode::FORBIDDEN,
        "M_FORBIDDEN",
        false,
    )
    .await;
    assert_eq!(
        get(&service, path, "native-admin").await.status_code,
        Some(StatusCode::OK)
    );
    mock.abort();
}
