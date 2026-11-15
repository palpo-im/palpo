use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use diesel::prelude::*;
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{OidcSession, Url};

async fn send(request: RequestBuilder, service: &Service) -> Response {
    let router = service.router();
    tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap()
}

async fn token_count() -> i64 {
    crate::data::schema::user_login_tokens::table
        .count()
        .get_result(&mut crate::data::db())
        .await
        .unwrap()
}

#[test]
fn sso_redirect_policy_matches_origins_and_exact_native_callbacks() {
    let allowed = vec![
        "https://client.example".to_owned(),
        "http://127.0.0.1".to_owned(),
        "http://[::1]".to_owned(),
        "element://vector/webapp".to_owned(),
    ];
    for destination in [
        "https://client.example/callback?state=abc",
        "http://127.0.0.1:49152/callback?state=abc",
        "http://[::1]:49153/callback",
        "element://vector/webapp?state=abc",
    ] {
        assert!(
            crate::config::sso_redirect_allowed(&Url::parse(destination).unwrap(), &allowed),
            "{destination}"
        );
    }
    for destination in [
        "https://client.example.evil.test/callback",
        "https://client.example:8443/callback",
        "https://evil.test/callback",
        "https://evil.test@client.example/callback",
        "http://127.0.0.2:49152/callback",
        "element://evil/webapp",
        "element://vector/other",
        "element://vector/webapp#changed",
        "element:vector/webapp",
        "javascript:alert(1)",
    ] {
        assert!(
            !crate::config::sso_redirect_allowed(&Url::parse(destination).unwrap(), &allowed),
            "{destination}"
        );
    }
    assert!(!crate::config::sso_redirect_allowed(
        &Url::parse("https://client.example/callback").unwrap(),
        &[],
    ));
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn http_oidc_redirect_requires_trusted_client() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn run() {
    crate::test_database::init();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let exchanges = Arc::new(AtomicUsize::new(0));
    let seen = exchanges.clone();
    let idp_origin = issuer.clone();
    let peer = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let count = stream.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&chunk[..count]);
                let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") else {
                    continue;
                };
                let header = String::from_utf8_lossy(&request[..end]);
                let length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            let response = if request.starts_with("GET /.well-known/openid-configuration ") {
                json!({
                    "issuer": idp_origin,
                    "authorization_endpoint": format!("{idp_origin}/authorize"),
                    "token_endpoint": format!("{idp_origin}/token"),
                    "userinfo_endpoint": format!("{idp_origin}/userinfo"),
                })
            } else if request.starts_with("POST /token ") {
                assert!(request.contains("code_verifier="), "{request}");
                seen.fetch_add(1, Ordering::SeqCst);
                json!({"access_token":"idp-token", "token_type":"Bearer", "expires_in":300})
            } else {
                assert!(request.starts_with("GET /userinfo "), "{request}");
                assert!(request.to_ascii_lowercase().contains("authorization: bearer idp-token"));
                json!({"sub":"subject-1", "preferred_username":"alice", "email":"alice@example.test", "email_verified":true})
            }.to_string();
            let header = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                response.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    crate::config::CONFIG.set(serde_json::from_value(json!({
        "server_name":"oidc-redirect.example", "db":{"url":"unused"},
        "oidc": {
            "enable":true,
            "sso_allowed_redirect_origins":["https://client.example", "http://127.0.0.1", "element://vector/webapp"],
            "providers":{"test": {
                "issuer":issuer, "client_id":"palpo-test", "client_secret":"test-secret",
                "redirect_uri":"https://matrix.example/_matrix/client/oidc/callback",
            }}
        }
    })).unwrap()).unwrap();
    let service = Service::new(crate::routing::root());
    let start_url = |target: &str| {
        let mut url = Url::parse("http://localhost/_matrix/client/oidc/auth").unwrap();
        url.query_pairs_mut()
            .append_pair("provider", "test")
            .append_pair("redirectUrl", target);
        url.to_string()
    };
    for target in ["https://evil.example/callback", "element://evil/webapp"] {
        let mut response = send(TestClient::get(start_url(target)), &service).await;
        assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
        assert!(!response.headers().contains_key("location"));
        assert!(!response.headers().contains_key("set-cookie"));
        assert_eq!(
            response.take_json::<Value>().await.unwrap()["errcode"],
            "M_FORBIDDEN"
        );
        assert_eq!(token_count().await, 0);
    }
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);

    // Both standard Matrix SSO entry points must lead to the same policy,
    // rather than giving an untrusted client a separate token issuer.
    for path in ["login/sso/redirect", "login/sso/redirect/test"] {
        let mut url = Url::parse(&format!("http://localhost/_matrix/client/v3/{path}")).unwrap();
        url.query_pairs_mut()
            .append_pair("redirectUrl", "https://evil.example/callback");
        let response = send(TestClient::get(url), &service).await;
        assert_eq!(response.status_code, Some(StatusCode::FOUND));
        let local_redirect = response.headers()["location"].to_str().unwrap();
        assert!(local_redirect.starts_with("/_matrix/client/oidc/auth?"));
        let response = send(
            TestClient::get(format!("http://localhost{local_redirect}")),
            &service,
        )
        .await;
        assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
        assert!(!response.headers().contains_key("location"));
        assert_eq!(token_count().await, 0);
    }
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);

    // Revalidate a cookie whose destination was changed after the login began.
    // A forged/legacy cookie must not reach even the IdP token endpoint.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let session = OidcSession {
        state: "state-for-denied-client".to_owned(),
        code_verifier: Some("verifier".to_owned()),
        provider: "test".to_owned(),
        redirect_url: Some("https://evil.example/callback".to_owned()),
        created_at: now,
    };
    let session_json = serde_json::to_string(&session).unwrap();
    let cookie = cookie::Cookie::new("oidc_session", session_json)
        .encoded()
        .to_string();
    let response = send(TestClient::get("http://localhost/_matrix/client/oidc/callback?code=valid-code&state=state-for-denied-client")
        .add_header("Cookie", cookie, true), &service).await;
    assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
    assert!(!response.headers().contains_key("location"));
    assert_eq!(token_count().await, 0);
    assert_eq!(exchanges.load(Ordering::SeqCst), 0);

    for target in [
        "https://client.example/callback?state=client-state",
        "http://127.0.0.1:49152/callback?state=desktop",
        "element://vector/webapp?state=native",
    ] {
        let response = send(TestClient::get(start_url(target)), &service).await;
        assert_eq!(response.status_code, Some(StatusCode::FOUND));
        let authorization = Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
        let state = authorization
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        assert!(
            authorization
                .query_pairs()
                .any(|(key, value)| key == "code_challenge_method" && value == "S256")
        );
        let cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let mut callback = Url::parse("http://localhost/_matrix/client/oidc/callback").unwrap();
        callback
            .query_pairs_mut()
            .append_pair("code", "valid-code")
            .append_pair("state", &state);
        let response = send(
            TestClient::get(callback).add_header("Cookie", cookie, true),
            &service,
        )
        .await;
        assert_eq!(response.status_code, Some(StatusCode::FOUND));
        let destination = Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
        let token = destination
            .query_pairs()
            .find(|(key, _)| key == "loginToken")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(destination.scheme(), Url::parse(target).unwrap().scheme());
        assert_eq!(destination.host(), Url::parse(target).unwrap().host());
        assert_eq!(destination.path(), Url::parse(target).unwrap().path());
        assert_eq!(token_count().await, 1);
        let mut response = send(
            TestClient::post("http://localhost/_matrix/client/v3/login")
                .json(&json!({"type":"m.login.token", "token":token})),
            &service,
        )
        .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        let login = response.take_json::<Value>().await.unwrap();
        assert_eq!(login["user_id"], "@alice_subject-1:oidc-redirect.example");
        assert!(
            login["access_token"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert_eq!(token_count().await, 0);
    }
    assert_eq!(exchanges.load(Ordering::SeqCst), 3);
    peer.abort();
}
