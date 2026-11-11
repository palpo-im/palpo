//! Isolated HTTP/PG contracts for advertised and explicitly unsupported capabilities.
use std::sync::{Arc, Mutex};

use salvo::conn::Acceptor;
use salvo::http::Method;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, UnixMillis, UserId};
use crate::{config, data};

const LOCAL: &str = "capabilities.example";
const TOKEN: &str = "client-secret-token";
const HS_TOKEN: &str = "dedicated-hs-token";

fn isolated<F: std::future::Future<Output = ()> + 'static>(run: fn() -> F) {
    std::thread::Builder::new()
        .name("capability-contract-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
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

async fn service() -> Service {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":LOCAL,"db":{"url":"unused"},"ip_range_denylist":[],
                "federation":{"enable":true},"delayed_events":{"enable":false},
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
            }))
            .unwrap(),
        )
        .unwrap();
    let owner = UserId::parse(format!("@owner:{LOCAL}")).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: owner.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "owner".into(),
        server_name: owner.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&owner).await.unwrap();
    data::user::device::create_device(&owner, &OwnedDeviceId::from("D"), TOKEN, None, None)
        .await
        .unwrap();
    Service::new(super::root())
}

async fn send(request: RequestBuilder, service: &Service) -> (StatusCode, Value) {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        let status = response.status_code.unwrap();
        (status, response.take_json().await.unwrap())
    })
    .await
    .unwrap()
}

fn authed(request: RequestBuilder) -> RequestBuilder {
    request.add_header("authorization", format!("Bearer {TOKEN}"), true)
}

async fn error(request: RequestBuilder, service: &Service, status: StatusCode, code: &str) {
    let (actual, body) = send(request, service).await;
    assert_eq!(actual, status, "{body}");
    assert_eq!(body["errcode"], code, "{body}");
}

#[test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
fn review_threepid_capabilities_and_rejections_match_routes() {
    isolated(threepid_contract);
}

async fn threepid_contract() {
    let service = service().await;
    let owner = UserId::parse(format!("@owner:{LOCAL}")).unwrap();
    data::user::replace_threepids(
        &owner,
        &[(
            "email".into(),
            "owner@example.org".into(),
            Some(10),
            Some(9),
        )],
    )
    .await
    .unwrap();
    let expected = json!({"threepids":[{"medium":"email","address":"owner@example.org","added_at":10,"validated_at":9}]});
    let (status, versions) = send(
        TestClient::get("http://localhost/_matrix/client/versions"),
        &service,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        versions["versions"].as_array().unwrap().last().unwrap(),
        "v1.12"
    );
    assert!(
        !versions["versions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "v1.19")
    );
    for (feature, built) in [
        (
            "org.continuwuity.msc4484.unstable",
            cfg!(feature = "unstable-msc4484"),
        ),
        (
            "org.continuwuity.presence_v2.msc4495",
            cfg!(feature = "unstable-msc4495"),
        ),
        (
            "org.continuwuity.presence_v2.msc4532",
            cfg!(feature = "unstable-msc4532"),
        ),
        (
            "org.continuwuity.presence_v2.msc4559",
            cfg!(feature = "unstable-msc4559"),
        ),
        ("org.matrix.msc4262", cfg!(feature = "unstable-msc4262")),
        ("uk.timedout.msc4494", cfg!(feature = "unstable-msc4494")),
    ] {
        assert_eq!(
            versions["unstable_features"].get(feature),
            built.then_some(&Value::Bool(true)),
            "{feature}"
        );
    }
    assert!(
        versions["unstable_features"]
            .get("org.matrix.msc4140")
            .is_none()
    );
    assert_eq!(
        versions["unstable_features"]["org.palpo.registration.email_otp"],
        false
    );
    for version in ["v3", "r0", "v1"] {
        let prefix = format!("http://localhost/_matrix/client/{version}");
        let (status, capabilities) = send(
            authed(TestClient::get(format!("{prefix}/capabilities"))),
            &service,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            capabilities["capabilities"]["m.3pid_changes"],
            json!({"enabled":false})
        );
        let (status, body) = send(
            authed(TestClient::get(format!("{prefix}/account/3pid"))),
            &service,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, expected);
        for endpoint in ["add", "bind", "unbind", "delete"] {
            error(authed(TestClient::post(format!("{prefix}/account/3pid/{endpoint}")).json(&json!({
                "medium":"email","address":"owner@example.org","id_server":"untrusted.example",
                "client_secret":"unused","sid":"unused"
            }))),&service,StatusCode::FORBIDDEN,"M_THREEPID_DENIED").await;
        }
        for medium in ["email", "msisdn"] {
            for path in [
                format!("account/3pid/{medium}/requestToken"),
                format!("account/{medium}/requestToken"),
            ] {
                // These are public refusal endpoints, including when no user token exists.
                error(
                    TestClient::post(format!("{prefix}/{path}")).json(&json!({
                        "client_secret":"unused","send_attempt":1,"email":"new@example.org",
                        "country":"GB","phone_number":"1234567","id_server":"untrusted.example"
                    })),
                    &service,
                    StatusCode::FORBIDDEN,
                    "M_THREEPID_DENIED",
                )
                .await;
            }
        }
        let (_, after) = send(
            authed(TestClient::get(format!("{prefix}/account/3pid"))),
            &service,
        )
        .await;
        assert_eq!(
            after, expected,
            "denied operations must retain stored contacts"
        );
    }
    for version in ["v1", "v2"] {
        let prefix = format!("http://localhost/_matrix/federation/{version}");
        for method in [Method::POST, Method::PUT] {
            let request = if method == Method::POST {
                TestClient::post(format!("{prefix}/3pid/onbind"))
            } else {
                TestClient::put(format!("{prefix}/3pid/onbind"))
            };
            error(request.json(&json!({"medium":"email","address":"owner@example.org","mxid":owner,"invites":[]})),&service,StatusCode::NOT_FOUND,"M_UNRECOGNIZED").await;
        }
        error(
            TestClient::put(format!(
                "{prefix}/exchange_third_party_invite/!room:{LOCAL}"
            ))
            .json(&json!({})),
            &service,
            StatusCode::UNAUTHORIZED,
            "M_MISSING_TOKEN",
        )
        .await;
    }
    let entries = data::user::get_threepids(&owner).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].address, "owner@example.org");
    assert_eq!(entries[0].added_at, UnixMillis(10));
    assert_eq!(entries[0].validated_at, UnixMillis(9));
}

#[derive(Clone)]
struct Peer {
    captured: Arc<Mutex<Vec<String>>>,
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
        assert_eq!(req.method(), Method::GET);
        assert_eq!(
            req.headers().get("authorization").unwrap(),
            format!("Bearer {HS_TOKEN}").as_str(),
            "the AS must receive its dedicated credential instead of client auth"
        );
        assert!(
            req.payload().await.unwrap().is_empty(),
            "client bodies must not be forwarded"
        );
        let query: Vec<_> =
            url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
        assert_eq!(
            query
                .iter()
                .filter(|(k, _)| k == "access_token")
                .map(|(_, v)| v.as_str())
                .collect::<Vec<_>>(),
            vec![HS_TOKEN]
        );
        assert!(!req.uri().to_string().contains(TOKEN));
        self.captured.lock().unwrap().push(req.uri().to_string());
        let path = req.uri().path();
        if path.ends_with("/protocol/irc") {
            res.render(Json(json!({"user_fields":["nickname"],"location_fields":["channel"],"icon":"","field_types":{},"instances":[]})));
        } else if path.contains("/location") {
            if path.ends_with("/irc") {
                assert!(query.iter().any(|(k, v)| k == "channel" && v == "#room"));
            } else {
                assert!(
                    query
                        .iter()
                        .any(|(k, v)| k == "alias" && v == "#portal:capabilities.example")
                );
            }
            res.render(Json(json!([{"alias":"#portal:capabilities.example","protocol":"irc","fields":{"channel":"#room"}}])));
        } else if path.ends_with("/user/irc") {
            assert!(
                query
                    .iter()
                    .any(|(k, v)| k == "nickname" && v == "Nick + &")
            );
            res.render(Json(json!([{"userid":"@portal:capabilities.example","protocol":"irc","fields":{"nickname":"Nick + &"}}])));
        } else if path.ends_with("/user") {
            assert!(
                query
                    .iter()
                    .any(|(k, v)| k == "userid" && v == "@portal:capabilities.example")
            );
            // Accept earlier Palpo AS payloads while emitting the standard wire key.
            res.render(Json(
                json!([{"user_id":"@portal:capabilities.example","protocol":"irc","fields":{}}]),
            ));
        } else {
            panic!("unexpected AS path: {path}");
        }
    }
}

#[test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
fn review_thirdparty_queries_track_live_appservice_disable() {
    isolated(thirdparty_contract);
}

async fn thirdparty_contract() {
    let service = service().await;
    let listener = TcpListener::new("127.0.0.1:0").bind().await;
    let address = listener.holdings()[0]
        .local_addr
        .clone()
        .into_std()
        .unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn(
        Server::new(listener).serve(Router::with_path("{*path}").get(Peer {
            captured: captured.clone(),
        })),
    );
    let registration: crate::core::appservice::Registration = serde_json::from_value(json!({
        "id":"bridge","url":format!("http://{address}"),"as_token":"as-token","hs_token":HS_TOKEN,
        "sender_localpart":"bridge","namespaces":{"users":[],"aliases":[],"rooms":[]},
        "protocols":["irc"]
    }))
    .unwrap();
    crate::appservice::register_appservice(registration)
        .await
        .unwrap();
    let endpoints = [
        "protocols",
        "protocol/irc",
        "location?alias=%23portal%3Acapabilities.example",
        "location/irc?channel=%23room&access_token=client-secret-token",
        "user?userid=%40portal%3Acapabilities.example",
        "user/irc?nickname=Nick+%2B+%26&access_token=client-secret-token",
    ];
    for disabled in [false, true, false] {
        assert!(
            crate::appservice::set_appservice_disabled("bridge", disabled)
                .await
                .unwrap()
        );
        let before = captured.lock().unwrap().len();
        for endpoint in endpoints {
            let (status, body) = send(
                authed(TestClient::get(format!(
                    "http://localhost/_matrix/client/v3/thirdparty/{endpoint}"
                )))
                .body("private client body"),
                &service,
            )
            .await;
            if disabled {
                if endpoint == "protocols" {
                    assert_eq!(status, StatusCode::OK);
                    assert_eq!(body, json!({}));
                } else {
                    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
                    assert_eq!(body["errcode"], "M_NOT_FOUND");
                }
            } else {
                assert_eq!(status, StatusCode::OK, "{body}");
                if endpoint == "protocols" {
                    assert!(body.get("irc").is_some());
                } else if endpoint.starts_with("protocol/") {
                    assert_eq!(body["user_fields"], json!(["nickname"]));
                } else {
                    assert_eq!(body.as_array().unwrap().len(), 1);
                    if endpoint.starts_with("user") {
                        assert_eq!(body[0]["userid"], "@portal:capabilities.example");
                        assert!(body[0].get("user_id").is_none());
                    }
                }
            }
        }
        assert_eq!(
            captured.lock().unwrap().len() - before,
            if disabled { 0 } else { 6 },
            "disabled services must not receive discovery queries"
        );
    }
    for endpoint in ["location", "user"] {
        error(
            authed(TestClient::get(format!(
                "http://localhost/_matrix/client/v3/thirdparty/{endpoint}"
            ))),
            &service,
            StatusCode::BAD_REQUEST,
            "M_MISSING_PARAM",
        )
        .await;
    }
    error(
        authed(TestClient::get(
            "http://localhost/_matrix/client/v3/thirdparty/protocol/not-registered",
        )),
        &service,
        StatusCode::NOT_FOUND,
        "M_NOT_FOUND",
    )
    .await;
    server.abort();
    let _ = server.await;
}
