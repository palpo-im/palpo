//! Real AS HTTP requests through the authenticated client preview route.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use salvo::conn::Acceptor;
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};

use super::*;

const HOST: &str = "https://93.184.216.34";
const TARGET: &str = "https://93.184.216.34/normal/private?token=private&x=1";
const EXCLUSIVE: &str = "https://93.184.216.34/exclusive/private";
const PATH: &str = "/bridge/_matrix/app/unstable/uk.half-shot.msc4417/preview_url";
const NORMAL_PATTERN: &str = r"https://93\.184\.216\.34/normal";
const FIRST_PATTERN: &str =
    r"https://93\.184\.216\.34/normal|https?://(blocked\.example|localhost|127\.0\.0\.1)/";

#[derive(Clone)]
enum Reply {
    Personal,
    Json(Value),
    Failure,
    Text(String),
    Redirect(String),
    Timeout,
}
#[derive(Clone)]
struct Captured {
    path: String,
    authorization: Option<String>,
    query: BTreeMap<String, String>,
}
#[derive(Clone)]
struct Peer {
    reply: Arc<Mutex<Reply>>,
    requests: Arc<Mutex<Vec<Captured>>>,
}
#[async_trait::async_trait]
impl Handler for Peer {
    async fn handle(&self, req: &mut Request, _: &mut Depot, res: &mut Response, _: &mut FlowCtrl) {
        let query: BTreeMap<String, String> =
            url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
        let user = query
            .get("user_id")
            .cloned()
            .unwrap_or_else(|| "anonymous".into());
        self.requests.lock().unwrap().push(Captured {
            path: req.uri().path().into(),
            authorization: req
                .headers()
                .get("authorization")
                .map(|h| h.to_str().unwrap().into()),
            query,
        });
        let reply = self.reply.lock().unwrap().clone();
        match reply {
            Reply::Personal => res.render(Json(json!({"og:title":user,"og:image":"mxc://preview.example/image","matrix:image:size":123}))),
            Reply::Json(value) => res.render(Json(value)),
            Reply::Failure => { res.status_code(StatusCode::SERVICE_UNAVAILABLE); res.render(Json(json!({"og:title":"failure payload"}))); }
            Reply::Text(value) => res.render(value),
            Reply::Redirect(url) => { res.status_code(StatusCode::FOUND); res.add_header("Location",url,true).unwrap(); }
            Reply::Timeout => { tokio::time::sleep(Duration::from_secs(3)).await; res.render(Json(json!({"og:title":"late"}))); }
        }
    }
}
struct RunningPeer {
    peer: Peer,
    task: tokio::task::JoinHandle<()>,
    origin: String,
    token: String,
}
impl RunningPeer {
    fn set(&self, reply: Reply) {
        *self.peer.reply.lock().unwrap() = reply;
    }
    fn count(&self) -> usize {
        self.peer.requests.lock().unwrap().len()
    }
    fn captured(&self) -> Vec<Captured> {
        self.peer.requests.lock().unwrap().clone()
    }
}
impl Drop for RunningPeer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn register(id: &str, pattern: &str, exclusive: bool) -> RunningPeer {
    let listener = TcpListener::new("127.0.0.1:0").bind().await;
    let address = listener.holdings()[0]
        .local_addr
        .clone()
        .into_std()
        .unwrap();
    let peer = Peer {
        reply: Arc::new(Mutex::new(Reply::Personal)),
        requests: Default::default(),
    };
    let task =
        tokio::spawn(Server::new(listener).serve(Router::with_path("{*path}").get(peer.clone())));
    let origin = format!("http://{address}/bridge");
    let token = format!("hs-{id}");
    let registration = serde_json::from_value(json!({
        "id":id,"url":origin,"as_token":format!("as-{id}"),"hs_token":token,
        "sender_localpart":format!("bot-{id}"),
        "namespaces":{"uk.half-shot.msc4417.preview_urls":[{"regex":pattern,"exclusive":exclusive}]}
    }))
    .unwrap();
    crate::appservice::register_appservice(registration)
        .await
        .unwrap();
    RunningPeer {
        peer,
        task,
        origin,
        token,
    }
}

async fn setup(enabled: bool) -> Service {
    crate::test_database::init();
    let mut config = json!({"server_name":"preview.example","db":{"url":"unused"},
        "ip_range_denylist":["127.0.0.0/8","::1/128"],
        "url_preview":{"domain_explicit_allowlist":["*"],"domain_explicit_denylist":["blocked.example"],"max_spider_size":1024},
        "rc_message":{"per_second":0.0,"burst":0}});
    if enabled {
        config["url_preview"]["appservice_providers"] = json!([
            {"id":"first"},{"id":"second","send_user_id":true},
            {"id":"exclusive","send_user_id":true},{"id":"last"}
        ]);
    }
    crate::config::CONFIG
        .set(serde_json::from_value(config).unwrap())
        .unwrap();
    for name in ["alice", "bob"] {
        let user = UserId::parse(format!("@{name}:preview.example")).unwrap();
        data::user::create_user(&data::user::NewDbUser {
            id: user.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: name.into(),
            server_name: user.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        data::user::device::create_device(&user, &OwnedDeviceId::from("D"), name, None, None)
            .await
            .unwrap();
    }
    Service::new(crate::routing::root())
}
async fn cached(url: &str) {
    data::media::set_url_preview(
        &UrlPreviewData {
            og_title: Some("public cache".into()),
            ..Default::default()
        }
        .into_new_db_url_preview(url),
    )
    .await
    .unwrap();
}
async fn set_namespace(id: &str, pattern: &str) {
    use diesel::sql_types::{Jsonb, Text};
    use horme::RunQueryDsl;
    diesel::sql_query("UPDATE appservice_registrations SET namespaces=$2 WHERE id=$1")
        .bind::<Text, _>(id)
        .bind::<Jsonb, _>(
            json!({"uk.half-shot.msc4417.preview_urls":[{"regex":pattern,"exclusive":false}]}),
        )
        .execute(&mut data::db())
        .await
        .unwrap();
}
async fn request(service: &Service, url: &str, token: &str) -> (StatusCode, Value) {
    let mut endpoint = Url::parse("http://localhost/_matrix/client/v1/media/preview_url").unwrap();
    endpoint.query_pairs_mut().append_pair("url", url);
    let router = service.router();
    let token = token.to_owned();
    tokio::spawn(async move {
        let mut response = TestClient::get(endpoint.as_str())
            .add_header("Authorization", format!("Bearer {token}"), true)
            .send(router)
            .await;
        (
            response.status_code.unwrap(),
            response.take_json().await.unwrap_or(Value::Null),
        )
    })
    .await
    .unwrap()
}
async fn title(service: &Service, url: &str, user: &str, expected: &str) {
    let (status, body) = request(service, url, user).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["og:title"], expected, "{body}");
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_appservice_previews_opt_in_policy_and_failures() {
    run(true);
}
#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_appservice_previews_disabled_uses_local_cache() {
    run(false);
}
fn run(enabled: bool) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let service = setup(enabled).await;
                    let first = register("first", FIRST_PATTERN, false).await;
                    let second = register("second", NORMAL_PATTERN, false).await;
                    let exclusive =
                        register("exclusive", r"https://93\.184\.216\.34/exclusive", true).await;
                    let last = register("last", r"https://93\.184\.216\.34/exclusive", false).await;
                    let unrelated = register("unconfigured", r".*", false).await;
                    cached(TARGET).await;
                    if !enabled {
                        title(&service, TARGET, "alice", "public cache").await;
                        assert_eq!(
                            first.count()
                                + second.count()
                                + exclusive.count()
                                + last.count()
                                + unrelated.count(),
                            0
                        );
                        return;
                    }
                    // A matching configured provider takes precedence over the local cache.
                    title(&service, TARGET, "alice", "anonymous").await;
                    assert_eq!(second.count(), 0);
                    first.set(Reply::Failure);
                    for user in ["alice", "bob", "alice"] {
                        title(&service, TARGET, user, &format!("@{user}:preview.example")).await;
                    }
                    assert_eq!(
                        second.count(),
                        3,
                        "delegated private previews entered a shared cache"
                    );
                    assert_eq!(
                        data::media::get_url_preview(TARGET)
                            .await
                            .unwrap()
                            .og_title
                            .as_deref(),
                        Some("public cache")
                    );
                    for invalid in [
                        Reply::Json(Value::Null),
                        Reply::Json(json!({})),
                        Reply::Text("not JSON".into()),
                        Reply::Json(json!({"og:image":"https://untrusted.example/private"})),
                        Reply::Json(json!({"og:image":"mxc://invalid"})),
                        Reply::Json(json!({"og:title":{"nested":"invalid"}})),
                        Reply::Json(json!({"og:title":123})),
                        Reply::Json(json!({"og:image:width":-1})),
                        Reply::Json(json!({"og:title":"x".repeat(2048)})),
                        Reply::Redirect(last.origin.clone()),
                        Reply::Timeout,
                    ] {
                        first.set(invalid);
                        title(&service, TARGET, "alice", "@alice:preview.example").await;
                    }
                    assert_eq!(
                        last.count(),
                        0,
                        "redirect forwarded private URL or credentials"
                    );
                    first.set(Reply::Failure);
                    second.set(Reply::Failure);
                    title(&service, TARGET, "alice", "public cache").await;
                    let unmatched = format!("{HOST}/unmatched");
                    cached(&unmatched).await;
                    let before = first.count() + second.count();
                    title(&service, &unmatched, "alice", "public cache").await;
                    assert_eq!(first.count() + second.count(), before);
                    // Exclusive failures and empty results never try another provider or local
                    // cache.
                    cached(EXCLUSIVE).await;
                    exclusive.set(Reply::Failure);
                    let (status, body) = request(&service, EXCLUSIVE, "alice").await;
                    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
                    exclusive.set(Reply::Json(Value::Null));
                    assert_eq!(
                        request(&service, EXCLUSIVE, "alice").await,
                        (StatusCode::OK, json!({}))
                    );
                    exclusive.set(Reply::Personal);
                    title(&service, EXCLUSIVE, "alice", "@alice:preview.example").await;
                    assert_eq!(last.count(), 0);
                    let before = first.count();
                    for denied in [
                        "https://blocked.example/private",
                        "http://127.0.0.1/private",
                        "http://localhost/private",
                        "file:///private",
                    ] {
                        assert_eq!(
                            request(&service, denied, "alice").await.0,
                            StatusCode::FORBIDDEN,
                            "{denied}"
                        );
                    }
                    assert_eq!(first.count(), before, "denied URL disclosed to appservice");
                    // The actual HTTP route shares the existing bounded preview admission pool.
                    let leases: Vec<_> = (0..MAX_PREVIEW_REQUESTS)
                        .map(|i| {
                            PREVIEW_REQUESTS
                                .try_enter(&format!("occupied-{i}"))
                                .unwrap()
                        })
                        .collect();
                    assert_eq!(
                        request(&service, TARGET, "alice").await.0,
                        StatusCode::TOO_MANY_REQUESTS
                    );
                    drop(leases);
                    first.set(Reply::Personal);
                    title(&service, TARGET, "alice", "anonymous").await;
                    crate::appservice::set_appservice_disabled("first", true)
                        .await
                        .unwrap();
                    crate::appservice::set_appservice_disabled("second", true)
                        .await
                        .unwrap();
                    title(&service, TARGET, "alice", "public cache").await;
                    crate::appservice::set_appservice_disabled("first", false)
                        .await
                        .unwrap();
                    title(&service, TARGET, "alice", "anonymous").await;
                    // Bad metadata in one configured service must not disable other
                    // providers, unrelated URLs or the ordinary local fallback.
                    crate::appservice::set_appservice_disabled("second", false)
                        .await
                        .unwrap();
                    set_namespace("second", "[").await;
                    title(&service, TARGET, "alice", "anonymous").await;
                    title(&service, &unmatched, "alice", "public cache").await;
                    set_namespace("first", "[").await;
                    set_namespace("second", NORMAL_PATTERN).await;
                    second.set(Reply::Personal);
                    title(&service, TARGET, "bob", "@bob:preview.example").await;
                    set_namespace("second", "[").await;
                    title(&service, TARGET, "alice", "public cache").await;
                    set_namespace("first", FIRST_PATTERN).await;
                    set_namespace("second", NORMAL_PATTERN).await;
                    assert_eq!(
                        unrelated.count(),
                        0,
                        "unconfigured provider received private previews"
                    );
                    for peer in [&first, &second, &exclusive] {
                        for captured in peer.captured() {
                            assert_eq!(captured.path, PATH);
                            assert_eq!(
                                captured.authorization,
                                Some(format!("Bearer {}", peer.token))
                            );
                            assert!(
                                captured
                                    .query
                                    .get("url")
                                    .is_some_and(|url| url == TARGET || url == EXCLUSIVE)
                            );
                            assert!(!captured.query.contains_key("access_token"));
                            assert!(!captured.query.values().any(|v| v == "alice" || v == "bob"));
                        }
                    }
                    assert!(
                        first
                            .captured()
                            .iter()
                            .all(|c| !c.query.contains_key("user_id"))
                    );
                    assert!(
                        second
                            .captured()
                            .iter()
                            .all(|c| c.query.contains_key("user_id"))
                    );
                });
        })
        .unwrap()
        .join()
        .unwrap();
}
