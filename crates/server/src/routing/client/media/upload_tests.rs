use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, UnixMillis, UserId};
use crate::{config, data, storage};

async fn send(request: RequestBuilder, service: &Service) -> Response {
    let router = service.router();
    tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap()
}

async fn json_ok(mut response: Response) -> Value {
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

fn upload(server: &str, id: &str, token: &str) -> RequestBuilder {
    TestClient::put(format!(
        "http://localhost/_matrix/media/v3/upload/{server}/{id}"
    ))
    .add_header("Authorization", format!("Bearer {token}"), true)
    .add_header("Content-Type", "text/html", true)
    .body("<script>parent.mediaScriptExecuted=true</script>")
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn http_reserved_media_enforces_owner_and_safe_download_headers() {
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
    let dir = tempfile::tempdir().unwrap();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name": "media.example", "db": {"url": "unused"}, "ip_range_denylist": [],
                "storage": {"backend": "fs", "root": dir.path().to_str().unwrap()}
            }))
            .unwrap(),
        )
        .unwrap();
    storage::init(&config::get().storage).unwrap();
    for name in ["alice", "bob"] {
        let user_id = UserId::parse(format!("@{name}:media.example")).unwrap();
        data::user::create_user(&data::user::NewDbUser {
            id: user_id.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: name.into(),
            server_name: user_id.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        data::user::device::create_device(&user_id, &OwnedDeviceId::from("D"), name, None, None)
            .await
            .unwrap();
    }
    let service = Service::new(crate::routing::root());
    let created = json_ok(
        send(
            TestClient::post("http://localhost/_matrix/media/v3/create").add_header(
                "Authorization",
                "Bearer alice",
                true,
            ),
            &service,
        )
        .await,
    )
    .await;
    let uri = created["content_uri"].as_str().unwrap();
    let id = uri.rsplit('/').next().unwrap();
    assert!(created["unused_expires_at"].as_u64().unwrap() > UnixMillis::now().0);
    assert_eq!(
        send(upload("media.example", id, "bob"), &service)
            .await
            .status_code,
        Some(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        send(upload("media.example", "not-created", "alice"), &service)
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    assert_eq!(
        send(upload("foreign.example", id, "alice"), &service)
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    let alice = UserId::parse("@alice:media.example").unwrap();
    data::media::reserve_upload(&config::get().server_name, "expired", &alice, UnixMillis(1))
        .await
        .unwrap();
    assert_eq!(
        send(upload("media.example", "expired", "alice"), &service)
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
    let (first, second) = tokio::join!(
        send(upload("media.example", id, "alice"), &service),
        send(upload("media.example", id, "alice"), &service)
    );
    assert_eq!(
        usize::from(first.status_code == Some(StatusCode::OK))
            + usize::from(second.status_code == Some(StatusCode::OK)),
        1
    );
    assert!(
        first.status_code == Some(StatusCode::CONFLICT)
            || second.status_code == Some(StatusCode::CONFLICT)
    );
    let saved = data::media::get_metadata(&config::get().server_name, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.created_by, Some(alice.clone()));
    for suffix in ["".to_owned(), "/evil.html".to_owned()] {
        let mut response = send(
            TestClient::get(format!(
                "http://localhost/_matrix/media/v3/download/media.example/{id}{suffix}"
            )),
            &service,
        )
        .await;
        if response.status_code != Some(StatusCode::OK) {
            panic!(
                "download {}: {}",
                response.status_code.unwrap_or(StatusCode::OK),
                response.take_string().await.unwrap()
            );
        }
        assert!(
            response.headers()["Content-Disposition"]
                .to_str()
                .unwrap()
                .starts_with("attachment")
        );
        assert_eq!(response.headers()["X-Content-Type-Options"], "nosniff");
    }
    // Original-file thumbnail fallback must not turn HTML into an inline document.
    let response = send(
        TestClient::get(format!(
            "http://localhost/_matrix/media/v3/thumbnail/media.example/{id}?width=409&height=409"
        )),
        &service,
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert!(
        response.headers()["Content-Disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert_eq!(response.headers()["X-Content-Type-Options"], "nosniff");
    let uploaded = json_ok(
        send(
            TestClient::post("http://localhost/_matrix/media/v3/upload?filename=safe.png")
                .add_header("Authorization", "Bearer alice", true)
                .add_header("Content-Type", "image/png", true)
                .body("not-a-real-png"),
            &service,
        )
        .await,
    )
    .await;
    let png_id = uploaded["content_uri"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    let response = send(
        TestClient::get(format!(
            "http://localhost/_matrix/media/v3/download/media.example/{png_id}"
        )),
        &service,
    )
    .await;
    assert!(
        response.headers()["Content-Disposition"]
            .to_str()
            .unwrap()
            .starts_with("inline")
    );
    assert_eq!(response.headers()["X-Content-Type-Options"], "nosniff");
}
