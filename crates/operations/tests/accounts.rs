use palpo_operations::{accounts, api, matrix::Matrix, now_ms, store::Store};
use reqwest::Method;
use salvo::{
    conn::Acceptor,
    prelude::*,
    test::{ResponseExt, TestClient},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Homeserver {
    room: Option<Vec<Value>>,
    events: BTreeMap<String, Value>,
    transactions: BTreeMap<String, (String, Value)>,
    timeline: Vec<Value>,
    users: BTreeMap<String, Value>,
    devices: BTreeMap<String, String>,
    registration_calls: usize,
    revoke_during_uiaa: bool,
    lose_registration: bool,
    lose_notification: bool,
    lose_create: bool,
}
impl Homeserver {
    fn state(&mut self, kind: &str, key: &str, content: Value) {
        let events = self.room.as_mut().unwrap();
        events.retain(|e| e["type"] != kind || e["state_key"] != key);
        events.push(json!({"type":kind,"state_key":key,"content":content}));
    }
    fn event(&mut self, content: Value, sender: &str) -> Value {
        let id = format!("$event_{}", self.events.len());
        let event =
            json!({"event_id":id,"type":"m.room.message","sender":sender,"content":content});
        self.events.insert(id, event.clone());
        self.timeline.push(event.clone());
        event
    }
}
fn decoded(s: &str) -> String {
    s.replace("%40", "@")
        .replace("%3A", ":")
        .replace("%3a", ":")
        .replace("%21", "!")
        .replace("%24", "$")
        .replace("%23", "#")
}
#[handler]
async fn matrix_fixture(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let path = decoded(req.uri().path());
    let token = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .strip_prefix("Bearer ")
        .unwrap_or_default()
        .to_owned();
    let body = if matches!(*req.method(), Method::POST | Method::PUT) {
        req.parse_json::<Value>().await.unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let shared = depot.get_typed::<Arc<Mutex<Homeserver>>>().unwrap();
    let mut s = shared.lock().unwrap();
    let actor = match token.as_str() {
        "bot-secret" => "@signup:example.test",
        "admin-secret" => "@admin:example.test",
        "owner-secret" => "@owner:example.test",
        "other-secret" => "@other:example.test",
        "new-session-secret" => "@alice:example.test",
        _ => "",
    };
    let mut status = StatusCode::OK;
    let value = if path == "/_matrix/client/v3/register" {
        s.registration_calls += 1;
        if body["auth"].is_null() {
            if s.revoke_during_uiaa {
                s.users.get_mut("@admin:example.test").unwrap()["admin"] = json!(false);
            }
            status = StatusCode::UNAUTHORIZED;
            json!({"session":"uiaa","flows":[{"stages":["m.login.registration_token"]}]})
        } else if body["auth"]["token"] != "registration-secret"
            || body["auth"]["session"] != "uiaa"
        {
            status = StatusCode::FORBIDDEN;
            json!({"errcode":"M_FORBIDDEN"})
        } else {
            assert!(body["admin"].is_null());
            assert!(body["password"].as_str().unwrap().len() >= 12);
            let id = format!("@{}:example.test", body["username"].as_str().unwrap());
            let device = body["device_id"].as_str().unwrap().to_owned();
            s.users.insert(id.clone(), json!({"admin":false}));
            s.devices.insert(id.clone(), device.clone());
            if s.lose_registration {
                s.lose_registration = false;
                status = StatusCode::BAD_GATEWAY;
                json!({"errcode":"lost_reply"})
            } else {
                json!({"user_id":id,"device_id":device,"access_token":"new-session-secret"})
            }
        }
    } else if actor.is_empty() {
        status = StatusCode::UNAUTHORIZED;
        json!({})
    } else if path.ends_with("/account/whoami") {
        json!({"user_id":actor})
    } else if path == "/_palpo/admin/v1/appservices" {
        if s.users
            .get(actor)
            .is_some_and(|u| u["admin"] == true && u["locked"] != true)
        {
            json!([])
        } else {
            status = StatusCode::FORBIDDEN;
            json!({})
        }
    } else if path.starts_with("/_palpo/admin/v2/users/") {
        assert_eq!(token, "admin-secret");
        s.users
            .get(path.rsplit('/').next().unwrap())
            .cloned()
            .unwrap_or_else(|| {
                status = StatusCode::NOT_FOUND;
                json!({})
            })
    } else if path.starts_with("/_palpo/admin/v1/whois/") {
        let device = s.devices.get(path.rsplit('/').next().unwrap()).cloned();
        json!({"devices":device.map(|d|json!({d:{}})).unwrap_or(json!({}))})
    } else if path.contains("/directory/room/") {
        if s.room.is_some() {
            json!({"room_id":"!accounts:example.test"})
        } else {
            status = StatusCode::NOT_FOUND;
            json!({})
        }
    } else if path.ends_with("/createRoom") {
        assert!(s.room.is_none());
        s.room = Some(Vec::new());
        s.state(
            "m.room.member",
            "@signup:example.test",
            json!({"membership":"join"}),
        );
        s.state(
            "m.room.member",
            "@admin:example.test",
            json!({"membership":"invite"}),
        );
        s.state("m.room.join_rules", "", json!({"join_rule":"invite"}));
        s.state(
            "m.room.history_visibility",
            "",
            body["initial_state"][0]["content"].clone(),
        );
        if s.lose_create {
            s.lose_create = false;
            status = StatusCode::BAD_GATEWAY;
            json!({})
        } else {
            json!({"room_id":"!accounts:example.test"})
        }
    } else if path.ends_with("/state") {
        json!(s.room)
    } else if path.contains("/state/m.room.history_visibility") {
        s.state("m.room.history_visibility", "", body);
        json!({"event_id":"$history"})
    } else if path.contains("/send/m.room.message/") {
        let txn = path.rsplit('/').next().unwrap();
        if let Some((id, original)) = s.transactions.get(txn) {
            assert_eq!(original, &body, "retry changed a Matrix transaction body");
            json!({"event_id":id})
        } else {
            let event = s.event(body.clone(), actor);
            let id = event["event_id"].as_str().unwrap().to_owned();
            s.transactions.insert(txn.to_owned(), (id.clone(), body));
            if s.lose_notification {
                s.lose_notification = false;
                status = StatusCode::BAD_GATEWAY;
                json!({})
            } else {
                json!({"event_id":id})
            }
        }
    } else if path.ends_with("/messages") {
        assert_eq!(req.query::<String>("dir").as_deref(), Some("f"));
        let start = req.query::<usize>("from").unwrap_or(0);
        let end = (start + 100).min(s.timeline.len());
        json!({"chunk":s.timeline[start..end],"end":end.to_string()})
    } else if path.contains("/event/") {
        s.events
            .get(path.rsplit('/').next().unwrap())
            .cloned()
            .unwrap_or_else(|| {
                status = StatusCode::NOT_FOUND;
                json!({})
            })
    } else if path.ends_with("/displayname") || path.ends_with("/logout") {
        json!({})
    } else {
        panic!("unhandled Matrix path: {path}");
    };
    res.status_code(status);
    res.render(Json(value));
}
fn config() -> accounts::Configuration {
    serde_json::from_value(json!({"botMxid":"@signup:example.test","botToken":"bot-secret","adminToken":"admin-secret","approvers":["@admin:example.test"],"passwordKey":"12".repeat(32),"registrationToken":"registration-secret"})).unwrap()
}
fn applicant(name: &str) -> Value {
    json!({"id":if name=="alice"{"a".repeat(32)}else{"b".repeat(32)},"receipt":"c".repeat(64),"username":name,"password":"signup fixture password!","displayName":"Test applicant","reason":"Fixture approval"})
}
struct Fixture {
    app: Arc<api::App>,
    service: Service,
    server: Arc<Mutex<Homeserver>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let server = Arc::new(Mutex::new(Homeserver::default()));
        server
            .lock()
            .unwrap()
            .users
            .insert("@admin:example.test".into(), json!({"admin":true}));
        server
            .lock()
            .unwrap()
            .users
            .insert("@other:example.test".into(), json!({"admin":true}));
        let router = Router::new()
            .hoop(affix_state::inject(server.clone()))
            .push(
                Router::with_path("{**rest}")
                    .get(matrix_fixture)
                    .post(matrix_fixture)
                    .put(matrix_fixture),
            );
        let acceptor = TcpListener::new("127.0.0.1:0").bind().await;
        let addr = acceptor.holdings()[0]
            .local_addr
            .clone()
            .into_std()
            .unwrap();
        let task = tokio::spawn(async move {
            Server::new(acceptor).serve(router).await;
        });
        let app = api::App::new(
            Matrix::new(
                &format!("http://{addr}"),
                "example.test".to_owned().try_into().unwrap(),
            )
            .unwrap(),
            Store::memory().unwrap(),
            "https://operations.test",
            900000,
        )
        .unwrap()
        .with_accounts(config())
        .unwrap();
        let service = Service::new(api::router(app.clone()));
        Self {
            app,
            service,
            server,
            task,
        }
    }
    async fn prepared() -> Self {
        let f = Self::new().await;
        accounts::tick(&f.app).await.unwrap();
        f.server.lock().unwrap().state(
            "m.room.member",
            "@admin:example.test",
            json!({"membership":"join"}),
        );
        f
    }
    async fn post(&self, path: &str, input: Value, bearer: Option<&str>) -> (StatusCode, Value) {
        let mut builder = TestClient::post(format!("https://operations.test{path}")).add_header(
            "host",
            "operations.test",
            true,
        );
        if let Some(bearer) = bearer {
            builder = builder.add_header("authorization", format!("Bearer {bearer}"), true);
        }
        let mut response = builder.json(&input).send(&self.service).await;
        (
            response.status_code.unwrap_or(StatusCode::OK),
            response.take_json().await.unwrap(),
        )
    }
    async fn row(&self, id: &str) -> Value {
        self.app.store.lock().await.read().unwrap()["accountAccess"]["requests"][id].clone()
    }
    async fn pending(&self, name: &str) -> Value {
        let input = applicant(name);
        let result = self
            .post("/api/account-requests", input.clone(), None)
            .await;
        assert_eq!(result.0, StatusCode::ACCEPTED, "{result:?}");
        accounts::tick(&self.app).await.unwrap();
        let row = self.row(input["id"].as_str().unwrap()).await;
        assert_eq!(row["status"], "pending");
        row
    }
    fn decision(&self, row: &Value, decision: &str) -> Value {
        json!({"msgtype":"m.text","body":decision,"org.octos.approval_response":{"request_id":row["id"],"source_event_id":row["sourceEventId"],"tool_args_digest":row["digest"],"decision":decision},"m.relates_to":{"m.in_reply_to":{"event_id":row["sourceEventId"]}}})
    }
    async fn session(&self, token: &str, services: Vec<&str>) -> String {
        let (status, value) = self
            .post(
                "/_palpo/miniapp/v1/session",
                json!({"appId":api::APP_ID,"bundleDigest":"d".repeat(64),"services":services}),
                Some(token),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["sessionToken"].as_str().unwrap().to_owned()
    }
    async fn call(&self, token: &str, service: &str, args: Value) -> (StatusCode, Value) {
        self.post(
            "/_palpo/miniapp/v1/call",
            json!({"service":service,"args":args}),
            Some(token),
        )
        .await
    }
    async fn due(&self, id: &str) {
        self.app
            .store
            .lock()
            .await
            .transaction(|s| {
                s["accountAccess"]["requests"][id]["nextAttemptAt"] = json!(0);
                Ok(())
            })
            .unwrap();
    }
    async fn restart(&mut self) {
        let snapshot = self.app.store.lock().await.read().unwrap();
        let mut store = Store::memory().unwrap();
        store
            .transaction(|s| {
                *s = snapshot;
                Ok(())
            })
            .unwrap();
        self.app = api::App::new(
            self.app.matrix.clone(),
            store,
            "https://operations.test",
            900000,
        )
        .unwrap()
        .with_accounts(config())
        .unwrap();
        self.service = Service::new(api::router(self.app.clone()));
    }
}

#[tokio::test]
async fn signup_requires_bound_native_verdict_then_erases_password() {
    let f = Fixture::prepared().await;
    let input = applicant("alice");
    let row = f.pending("alice").await;
    let id = row["id"].as_str().unwrap();
    assert_eq!(f.server.lock().unwrap().registration_calls, 0);
    let state = f.app.store.lock().await.read().unwrap().to_string();
    for secret in [
        "signup fixture password!",
        "registration-secret",
        "admin-secret",
        "bot-secret",
        &"c".repeat(64),
        &"12".repeat(32),
    ] {
        assert!(!state.contains(secret));
    }
    let session = f
        .session(
            "admin-secret",
            vec!["palpo.accounts.list", "palpo.accounts.open"],
        )
        .await;
    let opened = f
        .call(&session, "palpo.accounts.open", json!({"requestId":id}))
        .await;
    assert_eq!(opened.0, StatusCode::OK, "{opened:?}");
    assert_eq!(opened.1["eventId"], row["sourceEventId"]);
    assert_eq!(f.row(id).await["status"], "pending");
    f.server
        .lock()
        .unwrap()
        .event(f.decision(&row, "approve"), "@admin:example.test");
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["status"], "approved");
    assert_eq!(f.server.lock().unwrap().registration_calls, 0);
    accounts::tick(&f.app).await.unwrap();
    let done = f.row(id).await;
    assert_eq!(done["status"], "registered");
    assert!(done["password"].is_null());
    assert!(done["resultEventId"].is_string());
    assert_eq!(f.server.lock().unwrap().registration_calls, 2);
    assert_eq!(
        f.post("/api/account-requests", input.clone(), None).await.0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        f.post(
            "/api/account-requests/status",
            json!({"id":id,"receipt":input["receipt"]}),
            None
        )
        .await
        .1["request"]["status"],
        "registered"
    );
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.server.lock().unwrap().registration_calls, 2);
    assert!(
        !f.app
            .store
            .lock()
            .await
            .read()
            .unwrap()
            .to_string()
            .contains("new-session-secret")
    );
}

#[tokio::test]
async fn signup_reconciles_lost_create_notice_and_registration_across_restart() {
    let mut f = Fixture::new().await;
    f.server.lock().unwrap().lose_create = true;
    assert!(accounts::tick(&f.app).await.is_err());
    f.restart().await;
    accounts::tick(&f.app).await.unwrap();
    f.server.lock().unwrap().state(
        "m.room.member",
        "@admin:example.test",
        json!({"membership":"join"}),
    );
    let input = applicant("alice");
    let id = input["id"].as_str().unwrap();
    assert_eq!(
        f.post("/api/account-requests", input.clone(), None).await.0,
        StatusCode::ACCEPTED
    );
    f.server.lock().unwrap().lose_notification = true;
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["status"], "notification_pending");
    f.restart().await;
    f.due(id).await;
    accounts::tick(&f.app).await.unwrap();
    let row = f.row(id).await;
    assert_eq!(row["status"], "pending");
    assert_eq!(f.server.lock().unwrap().transactions.len(), 1);
    f.server
        .lock()
        .unwrap()
        .event(f.decision(&row, "approve"), "@admin:example.test");
    accounts::tick(&f.app).await.unwrap();
    f.server.lock().unwrap().lose_registration = true;
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["status"], "registering");
    f.restart().await;
    f.due(id).await;
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["status"], "registered");
    assert_eq!(f.server.lock().unwrap().registration_calls, 2);
}

#[tokio::test]
async fn signup_refuses_forged_and_revoked_approvals_and_private_room_changes() {
    let f = Fixture::prepared().await;
    let row = f.pending("alice").await;
    let id = row["id"].as_str().unwrap();
    let mut wrong = f.decision(&row, "approve");
    wrong["org.octos.approval_response"]["tool_args_digest"] = json!("forged");
    f.server.lock().unwrap().event(wrong, "@admin:example.test");
    f.server
        .lock()
        .unwrap()
        .event(f.decision(&row, "approve"), "@other:example.test");
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["status"], "pending");
    f.server.lock().unwrap().state(
        "m.room.member",
        "@intruder:example.test",
        json!({"membership":"join"}),
    );
    let cursor = f.app.store.lock().await.read().unwrap()["accountAccess"]["cursor"].clone();
    f.server
        .lock()
        .unwrap()
        .event(f.decision(&row, "approve"), "@admin:example.test");
    assert!(accounts::tick(&f.app).await.is_err());
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["accountAccess"]["cursor"],
        cursor
    );
    f.server.lock().unwrap().state(
        "m.room.member",
        "@intruder:example.test",
        json!({"membership":"leave"}),
    );
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["status"], "approved");
    f.server
        .lock()
        .unwrap()
        .users
        .get_mut("@admin:example.test")
        .unwrap()["admin"] = json!(false);
    assert!(accounts::tick(&f.app).await.is_err());
    assert_eq!(f.server.lock().unwrap().registration_calls, 0);
    assert_eq!(f.row(id).await["status"], "approved");
}

#[tokio::test]
async fn signup_does_not_adopt_an_existing_account_without_original_device_proof() {
    let f = Fixture::prepared().await;
    let row = f.pending("alice").await;
    let id = row["id"].as_str().unwrap();
    f.server
        .lock()
        .unwrap()
        .event(f.decision(&row, "approve"), "@admin:example.test");
    accounts::tick(&f.app).await.unwrap();
    f.app
        .store
        .lock()
        .await
        .transaction(|s| {
            s["accountAccess"]["requests"][id]["attempted"] = json!(true);
            Ok(())
        })
        .unwrap();
    f.server
        .lock()
        .unwrap()
        .users
        .insert("@alice:example.test".into(), json!({"admin":false}));
    f.server
        .lock()
        .unwrap()
        .devices
        .insert("@alice:example.test".into(), "different-device".into());
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["status"], "name_unavailable");
    assert!(f.row(id).await["password"].is_null());
    assert_eq!(f.server.lock().unwrap().registration_calls, 0);
}

#[tokio::test]
async fn signup_rechecks_approver_after_registration_challenge() {
    let f = Fixture::prepared().await;
    let row = f.pending("alice").await;
    let id = row["id"].as_str().unwrap();
    f.server
        .lock()
        .unwrap()
        .event(f.decision(&row, "approve"), "@admin:example.test");
    accounts::tick(&f.app).await.unwrap();
    f.server.lock().unwrap().revoke_during_uiaa = true;
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["lastError"], "account_approver_revoked");
    assert_eq!(f.server.lock().unwrap().registration_calls, 1);
    assert!(
        !f.server
            .lock()
            .unwrap()
            .users
            .contains_key("@alice:example.test")
    );
}

#[tokio::test]
async fn signup_rejection_expiry_and_history_upgrade_preserve_terminal_decisions() {
    let mut f = Fixture::prepared().await;
    let row = f.pending("alice").await;
    let id = row["id"].as_str().unwrap();
    f.server.lock().unwrap().state(
        "m.room.history_visibility",
        "",
        json!({"history_visibility":"joined"}),
    );
    f.restart().await;
    accounts::tick(&f.app).await.unwrap();
    let replaced = f.row(id).await;
    assert_ne!(replaced["sourceEventId"], row["sourceEventId"]);
    assert_eq!(
        replaced["supersededSourceEventIds"][0],
        row["sourceEventId"]
    );
    f.server
        .lock()
        .unwrap()
        .event(f.decision(&row, "approve"), "@admin:example.test");
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["status"], "pending");
    f.server
        .lock()
        .unwrap()
        .event(f.decision(&replaced, "deny"), "@admin:example.test");
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(id).await["status"], "rejected");
    assert!(f.row(id).await["password"].is_null());
    let bob = f.pending("bob").await;
    let bob_id = bob["id"].as_str().unwrap();
    f.app
        .store
        .lock()
        .await
        .transaction(|s| {
            s["accountAccess"]["requests"][bob_id]["expiresAt"] = json!(now_ms() - 1);
            Ok(())
        })
        .unwrap();
    f.server
        .lock()
        .unwrap()
        .event(f.decision(&bob, "approve"), "@admin:example.test");
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(f.row(bob_id).await["status"], "expired");
    assert!(f.row(bob_id).await["password"].is_null());
    assert_eq!(f.server.lock().unwrap().registration_calls, 0);
}

#[tokio::test]
async fn signup_navigation_requires_exact_grant_current_admin_and_original_card() {
    let f = Fixture::prepared().await;
    let row = f.pending("alice").await;
    let args = json!({"requestId":row["id"]});
    let readonly = f.session("admin-secret", vec!["palpo.accounts.list"]).await;
    assert_eq!(
        f.call(&readonly, "palpo.accounts.open", args.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for actor in ["owner-secret", "other-secret"] {
        let session = f.session(actor, vec!["palpo.accounts.open"]).await;
        assert_eq!(
            f.call(&session, "palpo.accounts.open", args.clone())
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    let session = f.session("admin-secret", vec!["palpo.accounts.open"]).await;
    assert_eq!(
        f.call(
            &session,
            "palpo.accounts.open",
            json!({"requestId":row["id"],"roomId":"!forged:example.test"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let event_id = row["sourceEventId"].as_str().unwrap();
    let source = f.server.lock().unwrap().events[event_id].clone();
    for key in ["tool_args_digest", "request_id", "tool_name"] {
        f.server.lock().unwrap().events.get_mut(event_id).unwrap()["content"]["org.octos.approval_request"]
            [key] = json!("forged");
        assert_eq!(
            f.call(&session, "palpo.accounts.open", args.clone())
                .await
                .0,
            StatusCode::CONFLICT
        );
        f.server
            .lock()
            .unwrap()
            .events
            .insert(event_id.into(), source.clone());
    }
    f.server.lock().unwrap().state(
        "m.room.encryption",
        "",
        json!({"algorithm":"m.megolm.v1.aes-sha2"}),
    );
    assert_eq!(
        f.call(&session, "palpo.accounts.open", args).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(f.server.lock().unwrap().registration_calls, 0);
}

#[tokio::test]
async fn signup_public_receipts_changes_origins_and_rate_limits_fail_closed() {
    let f = Fixture::new().await;
    let input = applicant("alice");
    assert_eq!(
        f.post("/api/account-requests", input.clone(), None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    accounts::tick(&f.app).await.unwrap();
    assert_eq!(
        f.post("/api/account-requests", input.clone(), None).await.0,
        StatusCode::ACCEPTED
    );
    let mut changed = input.clone();
    changed["password"] = json!("different password!");
    assert_eq!(
        f.post("/api/account-requests", changed, None).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.post(
            "/api/account-requests/status",
            json!({"id":input["id"],"receipt":"d".repeat(64)}),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let response = TestClient::post("https://operations.test/api/account-requests")
        .add_header("host", "operations.test", true)
        .add_header("origin", "https://foreign.test", true)
        .json(&input)
        .send(&f.service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
    for i in 0..30 {
        let mut response = TestClient::post("https://operations.test/api/account-requests")
            .add_header("host", "operations.test", true)
            .add_header("x-forwarded-for", format!("10.0.0.{i}"), true)
            .json(&input)
            .send(&f.service)
            .await;
        if i == 29 {
            assert_eq!(response.status_code, Some(StatusCode::TOO_MANY_REQUESTS));
            assert_eq!(
                response.take_json::<Value>().await.unwrap()["code"],
                "account_rate_limited"
            );
        }
    }
}
