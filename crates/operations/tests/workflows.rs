use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use palpo_hagency_contract::*;
use palpo_operations::matrix::Matrix;
use palpo_operations::store::Store;
use palpo_operations::workflow::Workflows;
use palpo_operations::{api, now_ms};
use salvo::conn::Acceptor;
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

fn authority(now: u64) -> Value {
    json!({
        "engagements":{"engagement_a":{"id":"engagement_a","server":"example.test","owner":"@provider:example.test","coordinator":"@coordinator:example.test",
            "registrationGeneration":1,"delegationRevision":1,"delegationExpiresAtMs":now+3600000,"state":"verified","allowSelfApproval":false,"coordinatorApprovalV1":true}},
        "resources":{"grant_a":{"id":"grant_a","serverEngagementId":"engagement_a","revision":1,"allocatedTokens":1000000,"eligibleManagers":["@manager:example.test"]}},
        "projects":{"existing_project":{"projectId":"existing_project","serverEngagementId":"engagement_a","revision":1,"owner":"@manager:example.test","resourceAllocations":["grant_a"],"state":"ready"}}
    })
}
fn project_request() -> Value {
    json!({"kind":"project","request":{"id":"request_project","revision":1,"serverEngagementId":"engagement_a","projectId":"new_project",
        "owner":"@manager:example.test","requester":"@manager:example.test","definitionDigest":"a".repeat(64),"resourceAllocations":["grant_a"]}})
}
fn agent_request() -> Value {
    json!({"kind":"agent","request":{"id":"request_agent","revision":1,"serverEngagementId":"engagement_a","projectId":"existing_project","projectRevision":1,
        "resourceAllocationId":"grant_a","projectOwner":"@manager:example.test","requester":"@manager:example.test","definitionDigest":"b".repeat(64),"requestedTokens":100000}})
}
fn approval(request: &Value, command_id: &str) -> Value {
    let now = now_ms();
    let mut value = json!({"context":{"version":1,"commandId":command_id,"serverEngagementId":"engagement_a","registrationGeneration":1,"delegationRevision":1,
        "actor":"@coordinator:example.test","issuedAtMs":now,"expiresAtMs":now+600000},"request":request["request"]});
    if request["kind"] == "agent" {
        value["allocatedTokens"] = json!(100000);
    }
    value
}

#[handler]
async fn matrix_stub(req: &mut salvo::Request, depot: &mut Depot, res: &mut Response) {
    let revoked = depot.get_typed::<Arc<AtomicBool>>().unwrap();
    let token = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or_default();
    let user = match token {
        "manager-token" => "manager",
        "coordinator-token" => "coordinator",
        "admin-token" => "admin",
        _ => "",
    };
    if user.is_empty() || revoked.load(Ordering::SeqCst) {
        res.status_code(StatusCode::UNAUTHORIZED);
        res.render(Json(
            json!({"errcode":"M_UNKNOWN_TOKEN","error":"never forward upstream credentials"}),
        ));
        return;
    }
    if req.uri().path().ends_with("whoami") {
        res.render(Json(json!({"user_id":format!("@{user}:example.test")})));
    } else if user == "admin" {
        res.render(Json(json!({"appservices":[]})));
    } else {
        res.status_code(StatusCode::FORBIDDEN);
        res.render(Json(json!({"errcode":"M_FORBIDDEN"})));
    }
}

struct Fixture {
    app: Arc<api::App>,
    service: Service,
    matrix_task: tokio::task::JoinHandle<()>,
    revoked: Arc<AtomicBool>,
    _directory: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.matrix_task.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let revoked = Arc::new(AtomicBool::new(false));
        let router = Router::new()
            .hoop(affix_state::inject(revoked.clone()))
            .push(Router::with_path("{**rest}").get(matrix_stub));
        let acceptor = TcpListener::new("127.0.0.1:0").bind().await;
        let addr = acceptor.holdings()[0]
            .local_addr
            .clone()
            .into_std()
            .unwrap();
        let matrix_task = tokio::spawn(async move {
            Server::new(acceptor).serve(router).await;
        });
        let matrix = Matrix::new(
            &format!("http://{addr}"),
            "example.test".to_owned().try_into().unwrap(),
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(&directory.path().join("admin.sqlite")).unwrap();
        store
            .transaction(|state| {
                let workflows = Workflows {
                    authority: serde_json::from_value(authority(now_ms()))?,
                    ..Default::default()
                };
                workflows.authority.validate(matrix.server())?;
                workflows.save(state)
            })
            .unwrap();
        let app = api::App::new(matrix, store, "https://operations.test", 900000).unwrap();
        let service = Service::new(api::router(app.clone()));
        Self {
            app,
            service,
            matrix_task,
            revoked,
            _directory: directory,
        }
    }
    async fn post(&self, operation: &str, token: &str, input: Value) -> (StatusCode, Value) {
        let mut response = TestClient::post(format!(
            "http://operations.test/_palpo/miniapp/v1/{operation}"
        ))
        .add_header("host", "operations.test", true)
        .add_header("authorization", format!("Bearer {token}"), true)
        .json(&input)
        .send(&self.service)
        .await;
        let status = response.status_code.unwrap_or(StatusCode::OK);
        let body = response.take_json::<Value>().await.unwrap();
        (status, body)
    }
    async fn session(&self, name: &str) -> String {
        let (status, body) = self
            .post(
                "session",
                &format!("{name}-token"),
                json!({"appId":api::APP_ID,"bundleDigest":"c".repeat(64),"services":api::SERVICES}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["sessionToken"].as_str().unwrap().to_owned()
    }
    async fn call(&self, token: &str, service: &str, args: Value) -> (StatusCode, Value) {
        self.post("call", token, json!({"service":service,"args":args}))
            .await
    }
}

#[tokio::test]
async fn real_http_sessions_project_and_agent_decisions_commit_outbox_without_claiming_ready() {
    let f = Fixture::new().await;
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let admin = f.session("admin").await;
    for (request, command) in [
        (project_request(), "project_approval"),
        (agent_request(), "agent_approval"),
    ] {
        let (status, result) = f
            .call(&manager, "palpo.inbox.submit", request.clone())
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        let id = result["action"]["id"].clone();
        let (status, replayed) = f
            .call(&manager, "palpo.inbox.submit", request.clone())
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(replayed["action"]["id"], id);
        let args = json!({"id":id,"decision":"approve","command":approval(&request,command)});
        for unauthorized in [&admin, &manager] {
            let (status, _) = f
                .call(unauthorized, "palpo.inbox.decide", args.clone())
                .await;
            assert!(matches!(
                status,
                StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
            ));
        }
        let (status, approved) = f
            .call(&coordinator, "palpo.inbox.decide", args.clone())
            .await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        assert_eq!(approved["action"]["state"], "approved");
        assert_eq!(approved["action"]["execution"], "pending");
        let (status, replayed) = f.call(&coordinator, "palpo.inbox.decide", args).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(replayed, approved);
    }
    let state = f.app.store.lock().await.read().unwrap();
    let workflows = Workflows::load(&state).unwrap();
    assert_eq!(workflows.actions.len(), 2);
    assert_eq!(workflows.outbox.len(), 2);
    assert_eq!(workflows.receipts.len(), 2);
    assert!(workflows.outbox.values().all(|c| c["state"] == "pending"));
    assert!(!serde_json::to_string(&state).unwrap().contains("-token"));
    assert!(
        !serde_json::to_string(&state)
            .unwrap()
            .contains(&coordinator)
    );
}

#[tokio::test]
async fn concurrent_conflicting_decisions_commit_exactly_one_outbound_command() {
    let f = Fixture::new().await;
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let request = agent_request();
    let (_, result) = f
        .call(&manager, "palpo.inbox.submit", request.clone())
        .await;
    let args = |command| json!({"id":result["action"]["id"],"decision":"approve","command":approval(&request,command)});
    let (a, b) = tokio::join!(
        f.call(&coordinator, "palpo.inbox.decide", args("first")),
        f.call(&coordinator, "palpo.inbox.decide", args("second"))
    );
    let statuses = [a.0, b.0];
    assert!(statuses.contains(&StatusCode::OK));
    assert!(statuses.contains(&StatusCode::CONFLICT));
    let workflows = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    assert_eq!(workflows.outbox.len(), 1);
    assert_eq!(workflows.receipts.len(), 1);
}

#[tokio::test]
async fn revoking_coordinator_binding_blocks_a_prepared_decision() {
    let f = Fixture::new().await;
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let request = agent_request();
    let (_, result) = f
        .call(&manager, "palpo.inbox.submit", request.clone())
        .await;
    f.app
        .store
        .lock()
        .await
        .transaction(|s| {
            s["rustWorkflows"]["authority"]["engagements"]["engagement_a"]["state"] =
                json!("revoked");
            Ok(())
        })
        .unwrap();
    let (status,_)=f.call(&coordinator,"palpo.inbox.decide",json!({"id":result["action"]["id"],"decision":"approve","command":approval(&request,"old")})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        Workflows::load(&f.app.store.lock().await.read().unwrap())
            .unwrap()
            .outbox
            .len(),
        0
    );
}

#[tokio::test]
async fn marking_seen_keeps_action_pending_and_snooze_is_per_recipient() {
    let f = Fixture::new().await;
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let (_, result) = f
        .call(&manager, "palpo.inbox.submit", project_request())
        .await;
    let id = result["action"]["id"].clone();
    let (status, seen) = f
        .call(&coordinator, "palpo.inbox.seen", json!({"id":id}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen["action"]["needsMyAction"], true);
    let (status, snoozed) = f
        .call(
            &coordinator,
            "palpo.inbox.snooze",
            json!({"id":id,"minutes":5}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(snoozed["snoozedUntil"].as_u64().unwrap() > now_ms() + 290000);
    let (_, inbox) = f
        .call(
            &coordinator,
            "palpo.inbox.list",
            json!({"view":"needs_action"}),
        )
        .await;
    assert_eq!(inbox["pendingCount"], 1);
    let (status, _) = f
        .call(&manager, "palpo.inbox.snooze", json!({"id":id,"minutes":5}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn host_grants_disconnect_and_matrix_revocation_are_enforced() {
    let f = Fixture::new().await;
    let (_,opened)=f.post("session","manager-token",json!({"appId":api::APP_ID,"bundleDigest":"c".repeat(64),"services":["palpo.inbox.list"]})).await;
    let token = opened["sessionToken"].as_str().unwrap();
    let (status, _) = f.call(token, "palpo.inbox.submit", project_request()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = f.post("disconnect", token, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = f.call(token, "palpo.inbox.list", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let token = f.session("manager").await; // Disconnect did not log Matrix out.
    f.revoked.store(true, Ordering::SeqCst);
    let (status, error) = f.call(&token, "palpo.inbox.list", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!error.to_string().contains("never forward"));
    let mut response = TestClient::post("http://operations.test/_palpo/miniapp/v1/session")
        .add_header("host", "operations.test", true)
        .add_header("origin", "http://operations.test", true)
        .add_header("authorization", "Bearer manager-token", true)
        .json(&json!({}))
        .send(&f.service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
    assert_eq!(
        response.take_json::<Value>().await.unwrap()["code"],
        "host_only"
    );
}

#[test]
fn sqlite_port_preserves_legacy_state_tables_and_rolls_back_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin.sqlite");
    let legacy = json!({"version":1,"fleets":{"legacy":{"registration":{"as_token":"fixture-secret"}}},"audit":[{"action":"old"}],"actionInbox":{"records":{"old":{"state":"approved"}},"notices":{},"rooms":{}},"futureUnknown":{"preserve":[1,2,3]}});
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE state(id INTEGER PRIMARY KEY,body TEXT NOT NULL); CREATE TABLE fleet_delivery(id TEXT);").unwrap();
    db.execute("INSERT INTO state VALUES(1,?1)", [legacy.to_string()])
        .unwrap();
    db.execute("INSERT INTO fleet_delivery VALUES('keep')", [])
        .unwrap();
    drop(db);
    {
        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.read().unwrap(), legacy);
        assert!(Store::open(&path).is_err());
        let before = store.read().unwrap();
        let result: palpo_operations::Result<()> = store.transaction(|state| {
            state["fleets"] = json!({});
            Err(palpo_operations::fail(409, "injected_failure"))
        });
        assert!(result.is_err());
        assert_eq!(store.read().unwrap(), before);
        store
            .transaction(|state| Workflows::default().save(state))
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let mut actual = store.read().unwrap();
    actual.as_object_mut().unwrap().remove("rustWorkflows");
    assert_eq!(actual, legacy);
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT id FROM fleet_delivery", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[test]
fn node_lock_and_wrong_server_binding_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin.sqlite");
    let lock = dir.path().join("admin.sqlite.lock");
    std::fs::write(&lock, "123\n").unwrap();
    assert!(Store::open(&path).is_err());
    assert!(lock.exists());
    std::fs::remove_file(lock).unwrap();
    let mut store = Store::open(&path).unwrap();
    store
        .bind("example.test", "https://matrix.example.test")
        .unwrap();
    assert!(
        store
            .bind("other.test", "https://matrix.example.test")
            .is_err()
    );
    assert_eq!(
        store.read().unwrap()["serverBinding"]["serverName"],
        "example.test"
    );
}

#[test]
fn decision_receipt_survives_restart_and_rejects_changed_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admin.sqlite");
    let now = now_ms();
    let actor: MatrixUserId = "@coordinator:example.test".to_owned().try_into().unwrap();
    let manager: MatrixUserId = "@manager:example.test".to_owned().try_into().unwrap();
    let request = agent_request();
    let mut command = approval(&request, "restart_command");
    command["context"]["issuedAtMs"] = json!(now);
    let id;
    {
        let mut store = Store::open(&path).unwrap();
        id = store
            .transaction(|state| {
                let mut w = Workflows {
                    authority: serde_json::from_value(authority(now))?,
                    ..Default::default()
                };
                let action = w.submit(serde_json::from_value(request.clone())?, &manager, now)?;
                let id = action["id"].as_str().unwrap().to_owned();
                w.approve(&id, command.clone(), &actor, now)?;
                w.save(state)?;
                Ok(id)
            })
            .unwrap();
    }
    let mut store = Store::open(&path).unwrap();
    store
        .transaction(|state| {
            let mut w = Workflows::load(state)?;
            w.approve(&id, command.clone(), &actor, now + 1)?;
            assert_eq!(w.outbox.len(), 1);
            w.save(state)
        })
        .unwrap();
    let before = store.read().unwrap();
    let mut changed = command;
    changed["allocatedTokens"] = json!(50000);
    let result = store.transaction(|state| {
        let mut w = Workflows::load(state)?;
        w.approve(&id, changed, &actor, now + 2)?;
        w.save(state)
    });
    assert_eq!(result.unwrap_err().code, "command_conflict");
    assert_eq!(store.read().unwrap(), before);
}

#[test]
fn authority_import_cannot_reset_bindings_by_removing_and_readding_them() {
    let server: ServerName = "example.test".to_owned().try_into().unwrap();
    let original = authority(now_ms());
    let mut workflows = Workflows::default();
    workflows
        .import_authority(serde_json::from_value(original.clone()).unwrap(), &server)
        .unwrap();
    assert_eq!(
        workflows
            .import_authority(Default::default(), &server)
            .unwrap_err()
            .code,
        "authority_removal_requires_tombstone"
    );
    let mut changed = original.clone();
    changed["engagements"]["engagement_a"]["coordinator"] = json!("@replacement:example.test");
    assert_eq!(
        workflows
            .import_authority(serde_json::from_value(changed.clone()).unwrap(), &server)
            .unwrap_err()
            .code,
        "authority_revision_conflict"
    );
    changed["engagements"]["engagement_a"]["delegationRevision"] = json!(2);
    workflows
        .import_authority(serde_json::from_value(changed.clone()).unwrap(), &server)
        .unwrap();
    assert_eq!(
        workflows
            .import_authority(serde_json::from_value(original).unwrap(), &server)
            .unwrap_err()
            .code,
        "authority_revision_conflict"
    );
    changed["engagements"]["engagement_a"]["state"] = json!("revoked");
    workflows
        .import_authority(serde_json::from_value(changed.clone()).unwrap(), &server)
        .unwrap();
    changed["engagements"]["engagement_a"]["state"] = json!("verified");
    assert_eq!(
        workflows
            .import_authority(serde_json::from_value(changed).unwrap(), &server)
            .unwrap_err()
            .code,
        "authority_revision_conflict"
    );
}

#[test]
fn same_hostname_engagements_keep_coordinator_and_resources_separate() {
    let now = now_ms();
    let server: ServerName = "example.test".to_owned().try_into().unwrap();
    let manager: MatrixUserId = "@manager:example.test".to_owned().try_into().unwrap();
    let first: MatrixUserId = "@coordinator:example.test".to_owned().try_into().unwrap();
    let second: MatrixUserId = "@coordinator2:example.test".to_owned().try_into().unwrap();
    let mut snapshot = authority(now);
    let mut engagement = snapshot["engagements"]["engagement_a"].clone();
    engagement["id"] = json!("engagement_b");
    engagement["coordinator"] = json!(second);
    snapshot["engagements"]["engagement_b"] = engagement;
    let mut resource = snapshot["resources"]["grant_a"].clone();
    resource["id"] = json!("grant_b");
    resource["serverEngagementId"] = json!("engagement_b");
    snapshot["resources"]["grant_b"] = resource;
    let mut workflows = Workflows::default();
    workflows
        .import_authority(serde_json::from_value(snapshot).unwrap(), &server)
        .unwrap();

    let mut request = project_request();
    request["request"]["serverEngagementId"] = json!("engagement_b");
    // A resource on the same homeserver still belongs to a different engagement.
    assert_eq!(
        workflows
            .submit(
                serde_json::from_value(request.clone()).unwrap(),
                &manager,
                now
            )
            .unwrap_err()
            .code,
        "resource_not_granted"
    );
    assert!(workflows.actions.is_empty());
    request["request"]["resourceAllocations"] = json!(["grant_b"]);
    let submitted = workflows
        .submit(
            serde_json::from_value(request.clone()).unwrap(),
            &manager,
            now,
        )
        .unwrap();
    let id = submitted["id"].as_str().unwrap();
    let mut command = approval(&request, "approve_second_engagement");
    command["context"]["issuedAtMs"] = json!(now);
    command["context"]["serverEngagementId"] = json!("engagement_b");
    assert!(workflows.approve(id, command.clone(), &first, now).is_err());
    assert!(workflows.outbox.is_empty());
    assert_eq!(
        workflows.list(&first, now, "needs_action", 0, 50).unwrap()["total"],
        0
    );
    assert_eq!(
        workflows.list(&second, now, "needs_action", 0, 50).unwrap()["total"],
        1
    );
    command["context"]["actor"] = json!(second);
    assert_eq!(
        workflows.approve(id, command, &second, now).unwrap()["state"],
        "approved"
    );
    assert_eq!(workflows.authority.engagements.len(), 2);
    assert_eq!(workflows.outbox.len(), 1);
    assert_eq!(
        workflows.outbox.values().next().unwrap()["serverEngagementId"],
        "engagement_b"
    );
}

#[test]
fn origins_require_tls_except_loopback_and_never_accept_credentials_or_paths() {
    for origin in [
        "http://example.test",
        "https://user:password@example.test",
        "https://example.test/path",
        "https://example.test?token=fixture",
        "https://example.test#fragment",
    ] {
        assert!(Matrix::new(origin, "example.test".to_owned().try_into().unwrap()).is_err());
        let matrix = Matrix::new(
            "https://matrix.example.test",
            "example.test".to_owned().try_into().unwrap(),
        )
        .unwrap();
        assert!(api::App::new(matrix, Store::memory().unwrap(), origin, 900000).is_err());
    }
    for origin in [
        "https://example.test",
        "http://127.0.0.1:8091",
        "http://[::1]:8091",
        "http://localhost:8091",
    ] {
        let matrix = Matrix::new(origin, "example.test".to_owned().try_into().unwrap()).unwrap();
        assert!(api::App::new(matrix, Store::memory().unwrap(), origin, 900000).is_ok());
    }
}
