use std::time::Duration;

use salvo::conn::Acceptor;
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use super::authed_router;
use crate::core::UnixMillis;
use crate::core::appservice::Registration;
use crate::data::user::DbUser;
use crate::{AuthedInfo, config};

struct Authenticate(AuthedInfo);

#[async_trait::async_trait]
impl Handler for Authenticate {
    async fn handle(&self, _: &mut Request, depot: &mut Depot, _: &mut Response, _: &mut FlowCtrl) {
        depot.insert_typed(self.0.clone());
    }
}

#[handler]
async fn bridge(req: &mut Request, res: &mut Response) {
    assert_eq!(req.method(), &salvo::http::Method::POST);
    assert_eq!(
        req.headers()
            .get(reqwest::header::AUTHORIZATION)
            .and_then(|header| header.to_str().ok()),
        Some("Bearer hs-token")
    );
    assert_eq!(
        req.query::<String>("access_token").as_deref(),
        Some("hs-token")
    );
    let body: Value = req.parse_json().await.unwrap();
    let transaction_id = body["transaction_id"].as_str().unwrap();
    let path = match transaction_id {
        "root" => "/_matrix/app/v1/ping",
        "prefixed" | "rejected" => "/relay/_matrix/app/v1/ping",
        _ => panic!("unexpected transaction ID {transaction_id}"),
    };
    assert_eq!(req.uri().path(), path);
    if transaction_id == "rejected" {
        res.status_code(StatusCode::NOT_FOUND);
    }
    res.render(Json(json!({})));
}

fn authenticated_bridge(url: String) -> AuthedInfo {
    let registration: Registration = serde_json::from_value(json!({
        "id": "bridge",
        "url": url,
        "as_token": "as-token",
        "hs_token": "hs-token",
        "sender_localpart": "bridge",
        "namespaces": {}
    }))
    .unwrap();
    AuthedInfo {
        user: DbUser {
            id: "@bridge:ping.example".try_into().unwrap(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: "bridge".into(),
            server_name: "ping.example".try_into().unwrap(),
            appservice_id: Some("bridge".into()),
            shadow_banned: false,
            consent_at: None,
            consent_version: None,
            consent_server_notice_sent: None,
            approved_at: None,
            approved_by: None,
            deactivated_at: None,
            deactivated_by: None,
            locked_at: None,
            locked_by: None,
            created_at: UnixMillis::now(),
            suspended_at: None,
        },
        user_device: None,
        access_token_id: None,
        appservice: Some(registration.try_into().unwrap()),
        oauth_scopes: None,
    }
}

#[tokio::test]
#[ignore = "owns process-global HTTP client configuration; run alone"]
async fn bridge_ping_uses_matrix_endpoint() {
    config::CONFIG
        .set(
            serde_json::from_value(json!({"server_name": "ping.example", "db": {"url": "unused"}}))
                .unwrap(),
        )
        .unwrap();

    let listener = TcpListener::new("127.0.0.1:0").bind().await;
    let address = listener.holdings()[0]
        .local_addr
        .clone()
        .into_std()
        .unwrap();
    let peer = tokio::spawn(Server::new(listener).serve(Router::with_path("{*path}").goal(bridge)));

    for (prefix, transaction_id, expected_status) in [
        ("", "root", StatusCode::OK),
        ("/relay/", "prefixed", StatusCode::OK),
        ("/relay/", "rejected", StatusCode::BAD_GATEWAY),
    ] {
        let auth = authenticated_bridge(format!("http://{address}{prefix}"));
        let router = Router::with_path("_matrix/client/v1")
            .hoop(Authenticate(auth))
            .push(authed_router());
        let mut response = tokio::time::timeout(
            Duration::from_secs(5),
            TestClient::post("http://localhost/_matrix/client/v1/appservice/bridge/ping")
                .json(&json!({"transaction_id": transaction_id}))
                .send(router),
        )
        .await
        .unwrap();
        let body: Value = response.take_json().await.unwrap();
        assert_eq!(response.status_code, Some(expected_status), "{body}");
        if expected_status == StatusCode::OK {
            assert!(body["duration_ms"].is_u64(), "{body}");
        } else {
            assert_eq!(body["errcode"], "M_BAD_STATUS");
            assert_eq!(body["status"], 404);
        }
    }
    peer.abort();
}
