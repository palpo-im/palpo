//! Actual AS HTTP/PG discovery across status failures and malformed peers.
use std::sync::{Arc, Mutex};

use salvo::conn::Acceptor;
use salvo::http::Method;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, UnixMillis, UserId};
use crate::{config, data};

const LOCAL: &str = "thirdparty-peers.example";
const CLIENT_TOKEN: &str = "private-client-token";

#[derive(Clone, Copy)]
enum Mode {
    StatusFailure,
    Malformed,
    Healthy,
    Empty,
    Extended,
}

struct Captured {
    method: Method,
    path: String,
    authorization: Option<String>,
    query: Vec<(String, String)>,
    body_length: usize,
}

#[derive(Clone)]
struct Peer {
    mode: Arc<Mutex<Mode>>,
    captured: Arc<Mutex<Vec<Captured>>>,
}

fn protocol(extended: bool, failing: bool) -> Value {
    let mut instance = json!({
        "desc":"IRC fixture","fields":{"network":"example"},"network_id":"example"
    });
    if extended {
        instance["instance_id"] = json!("bridge-specific-extension");
    }
    json!({
        "user_fields":["network","nickname"],"location_fields":["network","channel"],
        "icon":if failing {"mxc://untrusted.example/failure"} else {"mxc://healthy.example/icon"},
        "field_types":{
            "network":{"regexp":".+","placeholder":"example"},
            "nickname":{"regexp":".+","placeholder":"alice"},
            "channel":{"regexp":".+","placeholder":"#matrix"}
        },
        "instances":[instance]
    })
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
        let path = req.uri().path().to_owned();
        let query = url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        let request = Captured {
            method: req.method().clone(),
            path: path.clone(),
            authorization: req
                .headers()
                .get("authorization")
                .map(|v| v.to_str().unwrap().to_owned()),
            query,
            body_length: req.payload().await.unwrap().len(),
        };
        self.captured.lock().unwrap().push(request);
        let mode = *self.mode.lock().unwrap();
        if matches!(mode, Mode::Malformed) {
            res.render("not JSON");
            return;
        }
        let is_protocol = path.contains("/protocol/");
        if matches!(mode, Mode::Empty) && is_protocol {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(json!({"errcode":"M_NOT_FOUND"})));
            return;
        }
        if matches!(mode, Mode::StatusFailure) {
            // A valid success-shaped JSON body must not override HTTP failure.
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
        }
        let failing = matches!(mode, Mode::StatusFailure);
        if is_protocol {
            res.render(Json(protocol(matches!(mode, Mode::Extended), failing)));
        } else if matches!(mode, Mode::Empty) {
            res.render(Json(json!([])));
        } else if path.contains("/location") {
            res.render(Json(json!([{
                "alias":if failing {"#bad:thirdparty-peers.example"} else {"#portal:thirdparty-peers.example"},
                "protocol":"irc","fields":{"channel":"#matrix","network":"example"}
            }])));
        } else {
            res.render(Json(json!([{
                "userid":if failing {"@bad:thirdparty-peers.example"} else {"@portal:thirdparty-peers.example"},
                "protocol":"irc","fields":{"nickname":"Alice + &","network":"example"}
            }])));
        }
    }
}

struct RunningPeer {
    handler: Peer,
    token: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for RunningPeer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn register(id: &str, mode: Mode) -> RunningPeer {
    let listener = TcpListener::new("127.0.0.1:0").bind().await;
    let address = listener.holdings()[0]
        .local_addr
        .clone()
        .into_std()
        .unwrap();
    let handler = Peer {
        mode: Arc::new(Mutex::new(mode)),
        captured: Arc::new(Mutex::new(Vec::new())),
    };
    let task = tokio::spawn(
        Server::new(listener).serve(Router::with_path("{*path}").get(handler.clone())),
    );
    let token = format!("hs-only-{id}");
    crate::appservice::register_appservice(serde_json::from_value(json!({
        "id":id,"url":format!("http://{address}"),"as_token":format!("as-only-{id}"),"hs_token":token,
        "sender_localpart":format!("bridge-{id}"),"namespaces":{"users":[],"aliases":[],"rooms":[]},"protocols":["irc"]
    })).unwrap()).await.unwrap();
    RunningPeer {
        handler,
        token,
        task,
    }
}

async fn send(request: RequestBuilder, service: &Service) -> (StatusCode, Value) {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        (
            response.status_code.unwrap(),
            response.take_json().await.unwrap(),
        )
    })
    .await
    .unwrap()
}

const ENDPOINTS: [&str; 6] = [
    "protocols",
    "protocol/irc",
    "location?alias=%23portal%3Athirdparty-peers.example",
    "location/irc?channel=%23matrix&access_token=private-client-token&%61ccess_token=second-client-secret",
    "user?userid=%40portal%3Athirdparty-peers.example",
    "user/irc?nickname=Alice+%2B+%26&access_token=private-client-token&%61ccess_token=second-client-secret",
];

fn request(endpoint: &str) -> RequestBuilder {
    TestClient::get(format!(
        "http://localhost/_matrix/client/v3/thirdparty/{endpoint}"
    ))
    .add_header("authorization", format!("Bearer {CLIENT_TOKEN}"), true)
    .body("client request body must not be forwarded")
}

async fn successful(service: &Service, extended: bool) {
    for endpoint in ENDPOINTS {
        let (status, body) = send(request(endpoint), service).await;
        assert_eq!(status, StatusCode::OK, "{endpoint}: {body}");
        if endpoint == "protocols" || endpoint == "protocol/irc" {
            let metadata = if endpoint == "protocols" {
                &body["irc"]
            } else {
                &body
            };
            assert_eq!(metadata, &protocol(extended, false));
        } else {
            assert_eq!(
                body.as_array().unwrap().len(),
                1,
                "failed peer leaked results: {body}"
            );
            assert_eq!(body[0]["protocol"], "irc");
            if endpoint.starts_with("location") {
                assert_eq!(body[0]["alias"], "#portal:thirdparty-peers.example");
            } else {
                assert_eq!(body[0]["userid"], "@portal:thirdparty-peers.example");
            }
        }
    }
}

fn verify_requests(peer: &RunningPeer) {
    let requests = peer.handler.captured.lock().unwrap();
    assert!(!requests.is_empty());
    for request in requests.iter() {
        assert_eq!(request.method, Method::GET);
        assert_eq!(request.body_length, 0);
        assert_eq!(
            request.authorization.as_deref(),
            Some(format!("Bearer {}", peer.token).as_str())
        );
        assert_eq!(
            request
                .query
                .iter()
                .filter(|(key, _)| key == "access_token")
                .map(|(_, value)| value)
                .collect::<Vec<_>>(),
            vec![&peer.token]
        );
        assert!(!request.query.iter().any(
            |(_, value)| value.contains(CLIENT_TOKEN) || value.contains("second-client-secret")
        ));
        if request.path.ends_with("/location/irc") {
            assert!(
                request
                    .query
                    .iter()
                    .any(|(key, value)| key == "channel" && value == "#matrix")
            );
        } else if request.path.ends_with("/user/irc") {
            assert!(
                request
                    .query
                    .iter()
                    .any(|(key, value)| key == "nickname" && value == "Alice + &")
            );
        }
    }
}

#[test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
fn review_thirdparty_queries_isolate_failed_peers_and_preserve_instances() {
    std::thread::Builder::new()
        .name("thirdparty-peer-http".into())
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
    config::CONFIG.set(serde_json::from_value(json!({
        "server_name":LOCAL,"db":{"url":"unused"},"ip_range_denylist":[],"auto_join_rooms":[],
        "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
    })).unwrap()).unwrap();
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
    data::user::device::create_device(&owner, &OwnedDeviceId::from("D"), CLIENT_TOKEN, None, None)
        .await
        .unwrap();
    let service = Service::new(super::root());
    // Registration order is intentionally distinct from the deterministic ID
    // order: both failing peers precede the usable bridge during selection.
    let healthy = register("z-healthy", Mode::Healthy).await;
    let malformed = register("b-malformed", Mode::Malformed).await;
    let status = register("a-status", Mode::StatusFailure).await;
    successful(&service, false).await;
    for peer in [&status, &malformed, &healthy] {
        verify_requests(peer);
    }

    // Preserve an existing optional extension without requiring it from a
    // standard AS response, whose nonempty instances use network_id only.
    *healthy.handler.mode.lock().unwrap() = Mode::Extended;
    successful(&service, true).await;

    let before: Vec<_> = [&status, &malformed, &healthy]
        .iter()
        .map(|peer| peer.handler.captured.lock().unwrap().len())
        .collect();
    for endpoint in [
        "protocol/unknown",
        "location/unknown?channel=x",
        "user/unknown?nickname=x",
    ] {
        let (code, body) = send(request(endpoint), &service).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        assert_eq!(body["errcode"], "M_NOT_FOUND");
    }
    let after: Vec<_> = [&status, &malformed, &healthy]
        .iter()
        .map(|peer| peer.handler.captured.lock().unwrap().len())
        .collect();
    assert_eq!(before, after, "unknown protocols must not be forwarded");

    // With the usable bridge disabled, failures cannot manufacture an inventory
    // or a mapping, even though the 500 body has a valid success schema.
    crate::appservice::set_appservice_disabled("z-healthy", true)
        .await
        .unwrap();
    let healthy_requests = healthy.handler.captured.lock().unwrap().len();
    for endpoint in ENDPOINTS {
        let (code, body) = send(request(endpoint), &service).await;
        if endpoint == "protocols" {
            assert_eq!(code, StatusCode::OK);
            assert_eq!(body, json!({}));
        } else {
            assert_eq!(code, StatusCode::NOT_FOUND);
            assert_eq!(body["errcode"], "M_NOT_FOUND");
        }
    }
    assert_eq!(
        healthy_requests,
        healthy.handler.captured.lock().unwrap().len()
    );
    crate::appservice::set_appservice_disabled("z-healthy", false)
        .await
        .unwrap();
    successful(&service, true).await;

    *healthy.handler.mode.lock().unwrap() = Mode::Empty;
    for endpoint in ENDPOINTS {
        let (code, body) = send(request(endpoint), &service).await;
        if endpoint == "protocols" {
            assert_eq!(code, StatusCode::OK);
            assert_eq!(body, json!({}));
        } else {
            assert_eq!(code, StatusCode::NOT_FOUND);
            assert_eq!(body["errcode"], "M_NOT_FOUND");
        }
    }
    for peer in [&status, &malformed, &healthy] {
        verify_requests(peer);
    }
}
