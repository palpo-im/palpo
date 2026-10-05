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

#[path = "support/associations.rs"]
mod association_cases;
#[path = "support/creation.rs"]
mod creation_cases;
#[path = "support/fleet_admin.rs"]
mod fleet_admin_cases;
#[path = "support/lifecycle.rs"]
mod lifecycle_cases;
#[path = "support/notifications.rs"]
mod notification_cases;
#[path = "support/refusals.rs"]
mod refusal_cases;
#[path = "support/retirement.rs"]
mod retirement_cases;

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
    if association_cases::matrix(req, depot, res).await {
        return;
    }
    if creation_cases::matrix(req, depot, res).await {
        return;
    }
    let revoked = depot.get_typed::<Arc<AtomicBool>>().unwrap();
    let token = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or_default();
    if token == "fixture-as" {
        res.render(Json(json!([
            {"type":"m.room.member","state_key":"@provider:example.test","content":{"membership":"join"}},
            {"type":"m.room.member","state_key":req.query::<String>("user_id"),"content":{"membership":"join"}}
        ])));
        return;
    }
    let user = match token {
        "manager-token" | "manager-new-token" => "manager",
        "coordinator-token" => "coordinator",
        "admin-token" => "admin",
        "provider-token" => "provider",
        "notices-token" => "notices",
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
    rooms: Arc<std::sync::Mutex<creation_cases::Rooms>>,
    registrations: Arc<std::sync::Mutex<association_cases::Registrations>>,
    _directory: tempfile::TempDir,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.matrix_task.abort();
    }
}
impl Fixture {
    async fn machine(&self, fleet: &str, operation: &str, body: Value) -> (StatusCode, Value) {
        let mut response = TestClient::post(format!(
            "https://operations.test/api/fleet/v2/{fleet}/{operation}"
        ))
        .add_header("host", "operations.test", true)
        .add_header("authorization", "Bearer fixture-machine", true)
        .add_header("x-hagency-generation", "1", true)
        .json(&body)
        .send(&self.service)
        .await;
        (
            response.status_code.unwrap_or(StatusCode::OK),
            response.take_json::<Value>().await.unwrap(),
        )
    }
    async fn new() -> Self {
        Self::configured(false).await
    }
    async fn configured(notices: bool) -> Self {
        let revoked = Arc::new(AtomicBool::new(false));
        let rooms = Arc::new(std::sync::Mutex::new(creation_cases::Rooms::default()));
        let registrations = Arc::new(std::sync::Mutex::new(
            association_cases::Registrations::default(),
        ));
        let router = Router::new()
            .hoop(affix_state::inject(revoked.clone()))
            .hoop(affix_state::inject(rooms.clone()))
            .hoop(affix_state::inject(registrations.clone()))
            .push(
                Router::with_path("{**rest}")
                    .get(matrix_stub)
                    .post(matrix_stub)
                    .put(matrix_stub),
            );
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
        let app = api::App::new(matrix, store, "https://operations.test", 900000)
            .unwrap()
            .with_transport("https://operations.test", "https://relay.test")
            .unwrap()
            .with_association_admin("@admin:example.test".to_owned().try_into().unwrap())
            .unwrap();
        let app = if notices {
            app.with_notifications(notification_cases::config())
                .unwrap()
        } else {
            app
        };
        let app = app.with_retirement("admin-token".into()).unwrap();
        let service = Service::new(api::router(app.clone()));
        Self {
            app,
            service,
            matrix_task,
            revoked,
            rooms,
            registrations,
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
async fn coordinator_decision_delivers_definition_and_runtime_observations_without_a_second_approval()
 {
    use palpo_operations::digest;
    let f = Fixture::new().await;
    let fleet = format!("hf_{}", "b".repeat(32));
    let now = now_ms();
    f.app.store.lock().await.transaction(|state|{
        let mut authority=authority(now);
        let mut binding=authority["engagements"]["engagement_a"].take();
        binding["id"]=json!(fleet);
        binding["registrationGeneration"]=json!(7);
        authority["engagements"]=json!({&fleet:binding});
        authority["resources"]["grant_a"]["serverEngagementId"]=json!(fleet);
        authority["projects"]["existing_project"]["serverEngagementId"]=json!(fleet);
        Workflows{authority:serde_json::from_value(authority)?,..Default::default()}.save(state)?;
        state["fleets"][&fleet]=json!({"id":fleet,"registrationGeneration":7,"installation":"installed","state":"ready","ownerMxid":"@provider:example.test",
            "representativeMxid":format!("@{fleet}_representative:example.test"),"transport":{"mode":"outbound","generation":1,"token":"fixture-machine","sequence":0},
            "registration":{"hs_token":"fixture-relay"},"capabilities":{"coordinatorApprovalV1":true}});
        Ok(())
    }).unwrap();
    let definition = json!({"v":1,"fleetId":fleet,"requestId":"request_agent","targetProjectId":"existing_project",
        "targetRoomId":"!project:example.test","sourceRoomId":"!reception:example.test","sourceEventId":"$request",
        "ownerMxid":"@manager:example.test","requesterMxid":"@manager:example.test","ownerDmRoomId":"!private:example.test",
        "role":"developer","requestedTokens":100000,"agentDefinition":{"name":"Littlewhite","instructions":"Assist project owner"}});
    let mut request = agent_request();
    request["request"]["serverEngagementId"] = json!(fleet);
    request["request"]["definitionDigest"] = json!(digest(&definition).unwrap());
    request["definition"] = definition.clone();
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let (status, submitted) = f
        .call(&manager, "palpo.inbox.submit", request.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    let id = submitted["action"]["id"].as_str().unwrap();
    let mut command = approval(&request, "decision_live");
    command["context"]["serverEngagementId"] = json!(fleet);
    command["context"]["registrationGeneration"] = json!(7);
    let (status, decision) = f
        .call(
            &coordinator,
            "palpo.inbox.decide",
            json!({"id":id,"decision":"approve","command":command}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{decision}");
    let mut polled=TestClient::get(format!("https://operations.test/api/fleet/v2/{fleet}/poll?lane=work&consumer=01234567-0123-0123-0123-0123456789ab&wait=0"))
        .add_header("host","operations.test",true).add_header("authorization","Bearer fixture-machine",true).add_header("x-hagency-generation","1",true).send(&f.service).await;
    let lease = polled.take_json::<Value>().await.unwrap();
    assert_eq!(lease["delivery"]["payload"]["coordinatorApproval"], command);
    assert_eq!(
        lease["delivery"]["payload"]["agentDefinition"],
        definition["agentDefinition"]
    );
    assert_eq!(
        f.machine(
            &fleet,
            "ack",
            json!({"lane":"work","id":lease["delivery"]["id"],"token":lease["delivery"]["token"]})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(&manager, "palpo.inbox.get", json!({"id":id}))
            .await
            .1["action"]["execution"],
        "pending"
    );
    let receipt = json!({"kind":"receipt","commandId":"decision_live","commandDigest":digest(&json!({"operation":"coordinator_agent_approval","command":command})).unwrap(),
        "agentId":"en_littlewhite","state":"applied","registrationGeneration":7,"delegationRevision":1});
    let update = json!({"v":2,"generation":1,"sequence":1,"heartbeat":true,"coordinatorUpdates":[{"id":"command_decision_live","digest":digest(&receipt).unwrap(),"payload":receipt}]});
    let (status, result) = f.machine(&fleet, "updates", update.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        f.call(&manager, "palpo.inbox.get", json!({"id":id}))
            .await
            .1["action"]["execution"],
        "provisioning"
    );
    assert_eq!(
        f.machine(&fleet, "updates", update.clone()).await.0,
        StatusCode::OK
    );
    let mut altered = update.clone();
    altered["heartbeat"] = json!(false);
    assert_eq!(
        f.machine(&fleet, "updates", altered).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut observed = definition.clone();
    observed["engagementId"] = json!("en_littlewhite");
    observed["state"] = json!("active");
    observed["agentMxid"] = json!(format!("@{fleet}_en_littlewhite:example.test"));
    observed["allocatedTokens"] = json!(100000);
    observed["consumedTokens"] = Value::Null;
    observed["usageObservedAtMs"] = Value::Null;
    observed["usageEvidence"] = json!("host_attributed_lower_bound");
    observed["usageComplete"] = json!(false);
    observed["quotaPaused"] = json!(false);
    observed["bound"] = json!(true);
    observed["ready"] = json!(true);
    observed["fulfillment"] = json!({"phase":"complete","incomplete":false});
    observed["observedAt"] = json!(chrono::Utc::now().to_rfc3339());
    let ready = json!({"v":2,"generation":1,"sequence":2,"heartbeat":true,"statuses":[observed]});
    let (status, result) = f.machine(&fleet, "updates", ready.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let view = f
        .call(&manager, "palpo.inbox.get", json!({"id":id}))
        .await
        .1;
    assert_eq!(view["action"]["execution"], "ready", "{view}");
    assert!(view["action"]["result"]["consumedTokens"].is_null());
    assert_eq!(view["action"]["result"]["usageComplete"], false);
    assert_eq!(
        view["action"]["result"]["usageEvidence"],
        "host_attributed_lower_bound"
    );
    let (status, listed) = f.call(&manager, "palpo.requests.list", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["requests"][0]["execution"], "ready");
    assert_eq!(listed["requests"][0]["usable"], true);
    assert_eq!(listed["requests"][0]["usage"]["state"], "unknown");
    assert!(listed["requests"][0]["usage"]["consumedTokens"].is_null());
    assert_eq!(
        listed["requests"][0]["agentDefinition"]["name"],
        "Littlewhite"
    );
    assert!(!listed.to_string().contains("Assist project owner"));
    assert!(!listed.to_string().contains("fixture-machine"));
    let chat = json!({"requestId":format!("{fleet}:request_agent")});
    f.rooms.lock().unwrap().states.insert("!project:example.test".into(),json!([
        {"type":"m.room.member","state_key":"@manager:example.test","content":{"membership":"join"}},
        {"type":"m.room.member","state_key":observed["agentMxid"],"content":{"membership":"invite"}}
    ]));
    assert_eq!(
        f.call(&manager, "palpo.requests.open", chat.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    f.rooms
        .lock()
        .unwrap()
        .states
        .get_mut("!project:example.test")
        .unwrap()[1]["content"]["membership"] = json!("join");
    let (code, target) = f.call(&manager, "palpo.requests.open", chat.clone()).await;
    assert_eq!(code, StatusCode::OK, "{target}");
    assert_eq!(
        target,
        json!({"v":1,"requestId":format!("{fleet}:request_agent"),"account":"@manager:example.test","roomId":"!project:example.test","agentMxid":observed["agentMxid"]})
    );
    let mut forged = chat.clone();
    forged["roomId"] = json!("!other:example.test");
    assert_eq!(
        f.call(&manager, "palpo.requests.open", forged).await.0,
        StatusCode::BAD_REQUEST
    );
    let admin = f.session("admin").await;
    assert!(matches!(
        f.call(&admin, "palpo.requests.open", chat.clone()).await.0,
        StatusCode::CONFLICT | StatusCode::NOT_FOUND
    ));
    f.rooms
        .lock()
        .unwrap()
        .states
        .get_mut("!project:example.test")
        .unwrap()[0]["content"]["membership"] = json!("leave");
    assert_eq!(
        f.call(&manager, "palpo.requests.open", chat.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    f.rooms
        .lock()
        .unwrap()
        .states
        .get_mut("!project:example.test")
        .unwrap()[0]["content"]["membership"] = json!("join");
    // Reject changed-sequence retries and cross-engagement observations.
    let mut wrong = ready.clone();
    wrong["statuses"][0]["agentMxid"] = json!("@other:example.test");
    assert_eq!(
        f.machine(&fleet, "updates", wrong.clone()).await.0,
        StatusCode::CONFLICT
    );
    wrong["sequence"] = json!(3);
    assert_eq!(
        f.machine(&fleet, "updates", wrong).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["fleets"][&fleet]["transport"]["sequence"],
        2
    );
    let mut metered = ready.clone();
    metered["sequence"] = json!(3);
    metered["statuses"][0]["consumedTokens"] = json!(42);
    metered["statuses"][0]["usageObservedAtMs"] = json!(now_ms());
    assert_eq!(
        f.machine(&fleet, "updates", metered.clone()).await.0,
        StatusCode::OK
    );
    let listed = f.call(&manager, "palpo.requests.list", json!({})).await.1;
    assert_eq!(listed["requests"][0]["usage"]["state"], "current");
    assert_eq!(listed["requests"][0]["usage"]["consumedTokens"], 42);
    assert_eq!(listed["requests"][0]["usage"]["complete"], false);
    assert!(listed["requests"][0].get("remainingTokens").is_none());
    // A credential rotation cannot keep advertising a previous generation as live.
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            state["fleets"][&fleet]["transport"]["generation"] = json!(2);
            Ok(())
        })
        .unwrap();
    let listed = f.call(&manager, "palpo.requests.list", json!({})).await.1;
    assert_eq!(listed["requests"][0]["usable"], false);
    assert_eq!(listed["requests"][0]["execution"], "unknown");
    assert_eq!(listed["requests"][0]["usage"]["state"], "stale");
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            state["fleets"][&fleet]["transport"]["generation"] = json!(1);
            Ok(())
        })
        .unwrap();
    // An old frozen observation cannot be made fresh by delivering it again.
    let mut old = metered;
    old["sequence"] = json!(4);
    old["statuses"][0]["observedAt"] = json!("2020-01-01T00:00:00Z");
    assert_eq!(f.machine(&fleet, "updates", old).await.0, StatusCode::OK);
    let view = f
        .call(&manager, "palpo.inbox.get", json!({"id":id}))
        .await
        .1;
    assert_ne!(view["action"]["execution"], "ready");
    let listed = f.call(&manager, "palpo.requests.list", json!({})).await.1;
    assert_eq!(listed["requests"][0]["usable"], false);
    assert_eq!(listed["requests"][0]["usage"]["state"], "stale");
    assert_eq!(listed["requests"][0]["execution"], "unknown");
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["rustWorkflows"]["outbox"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn agent_and_project_lists_are_role_scoped_paginated_and_keep_pending_usage_unknown() {
    let f = Fixture::new().await;
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let provider = f.session("provider").await;
    let admin = f.session("admin").await;
    for id in ["request_agent", "another_agent"] {
        let mut request = agent_request();
        request["request"]["id"] = json!(id);
        let (status, submitted) = f
            .call(&manager, "palpo.inbox.submit", request.clone())
            .await;
        assert_eq!(status, StatusCode::OK, "{submitted}");
        let (status, result) = f.call(&coordinator, "palpo.inbox.decide", json!({
            "id":submitted["action"]["id"],"expectedRevision":1,"decision":"approve","commandId":format!("approve_{id}")
        })).await;
        assert_eq!(status, StatusCode::OK, "{result}");
    }
    for token in [&manager, &coordinator, &provider] {
        let (status, first) = f
            .call(token, "palpo.requests.list", json!({"limit":1}))
            .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["total"], 2);
        assert_eq!(first["requests"].as_array().unwrap().len(), 1);
        let row = &first["requests"][0];
        assert_eq!(row["state"], "approved");
        assert_eq!(row["execution"], "pending");
        assert_eq!(row["usable"], false);
        assert!(row["allocatedTokens"].is_null());
        assert_eq!(row["usage"]["state"], "unknown");
        assert!(row["usage"]["consumedTokens"].is_null());
        let second = f
            .call(token, "palpo.requests.list", json!({"limit":1,"offset":1}))
            .await
            .1;
        assert_ne!(first["requests"][0]["id"], second["requests"][0]["id"]);
        let projects = f.call(token, "palpo.projects.list", json!({})).await.1;
        assert_eq!(projects["total"], 1);
        assert_eq!(projects["projects"][0]["canRequest"], false);
    }
    for service in ["palpo.projects.list", "palpo.requests.list"] {
        assert_eq!(f.call(&admin, service, json!({})).await.1["total"], 0);
        for args in [
            json!({"limit":0}),
            json!({"limit":101}),
            json!({"owner":"@manager:example.test"}),
        ] {
            assert_eq!(
                f.call(&manager, service, args).await.0,
                StatusCode::BAD_REQUEST
            );
        }
    }
    // Delegation removal revokes coordinator access without erasing the owner's history.
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            let mut workflows = Workflows::load(state)?;
            workflows
                .authority
                .engagements
                .get_mut("engagement_a")
                .unwrap()
                .delegation_expires_at_ms = 0;
            workflows.save(state)
        })
        .unwrap();
    assert_eq!(
        f.call(&coordinator, "palpo.requests.list", json!({}))
            .await
            .1["total"],
        0
    );
    assert_eq!(
        f.call(&manager, "palpo.requests.list", json!({})).await.1["total"],
        2
    );
}

#[tokio::test]
async fn token_top_up_form_binds_current_allocation_and_replays_after_execution() {
    let f = Fixture::new().await;
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let admin = f.session("admin").await;
    let (_, submitted) = f
        .call(&manager, "palpo.inbox.submit", agent_request())
        .await;
    let agent_id = submitted["action"]["id"].as_str().unwrap().to_owned();
    let (status, _) = f
        .call(
            &coordinator,
            "palpo.inbox.decide",
            json!({"id":agent_id,"expectedRevision":1,
        "decision":"approve","commandId":"original_agent"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let intent = json!({"kind":"token_top_up","agentActionId":agent_id,"requestId":"more_tokens",
        "expectedAllocatedTokens":100000,"requestedAdditionalTokens":"20000"});
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", intent.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    f.app.store.lock().await.transaction(|state| {
        state["fleets"]["engagement_a"] = json!({"id":"engagement_a","registrationGeneration":1,
            "state":"ready","installation":"installed","capabilities":{"coordinatorApprovalV1":true},
            "transport":{"mode":"outbound","generation":1}});
        let mut workflows=Workflows::load(state)?;
        workflows.observations.insert(agent_id.clone(),json!({"state":"active","engagementId":"en_native_agent",
            "generation":1,"allocatedTokens":100000,"quotaPaused":true,"observedAt":chrono::Utc::now().to_rfc3339(),"receivedAtMs":now_ms()}));
        workflows.save(state)
    }).unwrap();
    assert_eq!(
        f.call(&manager, "palpo.requests.list", json!({})).await.1["requests"][0]["canRequestTopUp"],
        true
    );
    for other in [&coordinator, &admin] {
        assert_eq!(
            f.call(other, "palpo.inbox.submit", intent.clone()).await.0,
            StatusCode::CONFLICT
        );
    }
    for tokens in ["-1", "0", "1e3", "9007199254740992", "20000.0", " 20000"] {
        let mut invalid = intent.clone();
        invalid["requestedAdditionalTokens"] = json!(tokens);
        assert_eq!(
            f.call(&manager, "palpo.inbox.submit", invalid).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let mut too_many = intent.clone();
    too_many["requestedAdditionalTokens"] = json!("1000000");
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", too_many).await.0,
        StatusCode::CONFLICT
    );
    let (status, submitted) = f.call(&manager, "palpo.inbox.submit", intent.clone()).await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    let top_up_id = submitted["action"]["id"].as_str().unwrap();
    assert_eq!(
        submitted["action"]["request"]["request"]["agentAllocationId"],
        "en_native_agent"
    );
    assert_eq!(
        submitted["action"]["request"]["request"]["requester"],
        "@manager:example.test"
    );
    let decision = json!({"id":top_up_id,"expectedRevision":1,"decision":"approve","commandId":"top_up_decision","reason":"Within quota"});
    let (status, approved) = f.call(&coordinator, "palpo.inbox.decide", decision).await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["action"]["decision"]["reason"], "Within quota");
    let state = f.app.store.lock().await.read().unwrap();
    let record = &state["rustWorkflows"]["outbox"]["top_up_decision"];
    assert_eq!(record["queued"], true);
    assert_eq!(record["command"]["additionalTokens"], 20000);
    assert_eq!(
        record["command"]["request"]["expectedAllocatedTokens"],
        100000
    );
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            let mut workflows = Workflows::load(state)?;
            workflows.observations.get_mut(&agent_id).unwrap()["allocatedTokens"] = json!(120000);
            workflows.actions.get_mut(top_up_id).unwrap().execution = "done".into();
            workflows.save(state)
        })
        .unwrap();
    let (status, replayed) = f.call(&manager, "palpo.inbox.submit", intent.clone()).await;
    assert_eq!(status, StatusCode::OK, "{replayed}");
    assert_eq!(replayed["action"]["execution"], "done");
    let mut changed = intent.clone();
    changed["requestedAdditionalTokens"] = json!("30000");
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", changed).await.0,
        StatusCode::CONFLICT
    );
    let mut fresh = intent;
    fresh["requestId"] = json!("another_top_up");
    fresh["expectedAllocatedTokens"] = json!(120000);
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            let mut workflows = Workflows::load(state)?;
            workflows.observations.get_mut(&agent_id).unwrap()["observedAt"] =
                json!("2020-01-01T00:00:00Z");
            workflows.save(state)
        })
        .unwrap();
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", fresh).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.call(&manager, "palpo.requests.list", json!({})).await.1["requests"][0]["canRequestTopUp"],
        false
    );
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
async fn octoscript_decision_intent_builds_authority_on_the_server_and_top_up_is_replayable() {
    let f = Fixture::new().await;
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let admin = f.session("admin").await;
    let (_, submitted) = f
        .call(&manager, "palpo.inbox.submit", agent_request())
        .await;
    let agent_id = submitted["action"]["id"].as_str().unwrap().to_owned();
    let args = json!({"id":agent_id,"expectedRevision":1,"decision":"approve","reason":"Within the project allocation","commandId":"miniapp_agent"});
    for unauthorized in [&manager, &admin] {
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
    assert_eq!(approved["action"]["execution"], "pending");
    assert_eq!(approved["action"]["kind"], "agent");
    let (status, repeated) = f
        .call(&coordinator, "palpo.inbox.decide", args.clone())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repeated, approved);
    // A temporarily offline Hagency must not require another human decision
    // after ten minutes. The command is bounded by the owner's delegation.
    let workflows = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    let command: AgentApproval =
        serde_json::from_value(workflows.outbox["miniapp_agent"]["command"].clone()).unwrap();
    let binding = &workflows.authority.engagements["engagement_a"];
    assert_eq!(
        command.context.expires_at_ms,
        binding.delegation_expires_at_ms
    );
    assert!(
        authorize_agent_approval(
            &command,
            &command.request,
            binding,
            &workflows.authority.projects["existing_project"],
            &command.context.actor,
            command.context.issued_at_ms + 601000
        )
        .is_ok()
    );
    assert!(
        authorize_agent_approval(
            &command,
            &command.request,
            binding,
            &workflows.authority.projects["existing_project"],
            &command.context.actor,
            binding.delegation_expires_at_ms
        )
        .is_err()
    );
    let mut changed = args;
    changed["reason"] = json!("different intent");
    assert_eq!(
        f.call(&coordinator, "palpo.inbox.decide", changed).await.0,
        StatusCode::CONFLICT
    );
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            let mut workflows = Workflows::load(state)?;
            workflows.observations.insert(
                agent_id.clone(),
                json!({"engagementId":"agent_native","state":"active","allocatedTokens":100000}),
            );
            state["fleets"]["engagement_a"] = json!({"id":"engagement_a","installation":"installed","state":"ready",
                "transport":{"mode":"outbound","generation":1,"token":"fixture-machine"},"capabilities":{"coordinatorApprovalV1":true}});
            workflows.save(state)
        })
        .unwrap();
    let definition = json!({"agentAllocationId":"agent_native","expectedAllocatedTokens":100000,"requestedAdditionalTokens":20000});
    let request = json!({"kind":"token_top_up","request":{"id":"topup_one","revision":1,"serverEngagementId":"engagement_a","projectId":"existing_project","projectRevision":1,
        "resourceAllocationId":"grant_a","projectOwner":"@manager:example.test","requester":"@manager:example.test","agentAllocationId":"agent_native",
        "definitionDigest":palpo_operations::digest(&definition).unwrap(),"expectedAllocatedTokens":100000,"requestedAdditionalTokens":20000},"definition":definition});
    let (status, submitted) = f
        .call(&manager, "palpo.inbox.submit", request.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    let args = json!({"id":submitted["action"]["id"],"expectedRevision":1,"commandId":"miniapp_topup","decision":"approve","reason":"Approved"});
    let (status, result) = f
        .call(&coordinator, "palpo.inbox.decide", args.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        f.call(&coordinator, "palpo.inbox.decide", args).await.0,
        StatusCode::OK
    );
    let state = f.app.store.lock().await.read().unwrap();
    let workflows = Workflows::load(&state).unwrap();
    assert_eq!(workflows.outbox.len(), 2);
    assert_eq!(
        workflows.outbox["miniapp_topup"]["command"]["additionalTokens"],
        20000
    );
    assert_eq!(
        workflows.outbox["miniapp_topup"]["command"]["context"]["actor"],
        "@coordinator:example.test"
    );
    assert_eq!(
        workflows.observations[&agent_id]["allocatedTokens"], 100000,
        "only Hagency can apply the quota increase"
    );
}

#[tokio::test]
async fn existing_manifest_negotiates_only_implemented_services() {
    let f = Fixture::new().await;
    let input = json!({"appId":api::APP_ID,"bundleDigest":"c".repeat(64),"services":["palpo.session.open","palpo.inbox.list","palpo.fleets.register"]});
    let (status, opened) = f.post("session", "manager-token", input.clone()).await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    assert_eq!(
        opened["services"],
        json!(["palpo.session.open", "palpo.inbox.list"])
    );
    let token = opened["sessionToken"].as_str().unwrap();
    assert_eq!(
        f.call(
            token,
            "palpo.fleets.register",
            json!({"fleetId":"engagement_a"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut unsupported = input;
    unsupported["services"] = json!(["palpo.session.open", "palpo.grant_everything"]);
    assert_eq!(
        f.post("session", "manager-token", unsupported).await.0,
        StatusCode::FORBIDDEN
    );
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

#[tokio::test]
async fn matrix_relay_poll_and_ack_use_the_real_rust_http_routes() {
    let f = Fixture::new().await;
    let id = format!("hf_{}", "a".repeat(32));
    f.app.store.lock().await.transaction(|state|{
        state["fleets"][&id]=json!({"id":id,"installation":"installed","state":"pending_connection","transport":{"mode":"outbound","generation":1,"token":"fixture-machine"},"registration":{"hs_token":"fixture-relay"}});Ok(())
    }).unwrap();
    let mut relay=TestClient::put(format!("https://relay.test/api/relay/v2/{id}/_matrix/app/v1/transactions/t1"))
        .add_header("host","relay.test",true).add_header("authorization","Bearer fixture-relay",true)
        .json(&json!({"events":[{"type":"test.event","content":{"a":0.5,"10":10,"2":2,"😀":true,"":false}}]})).send(&f.service).await;
    assert_eq!(relay.status_code.unwrap_or(StatusCode::OK), StatusCode::OK);
    assert_eq!(relay.take_json::<Value>().await.unwrap(), json!({}));
    let url = format!(
        "https://operations.test/api/fleet/v2/{id}/poll?lane=matrix&consumer=01234567-0123-0123-0123-0123456789ab&wait=0"
    );
    let mut response = TestClient::get(&url)
        .add_header("host", "operations.test", true)
        .add_header("authorization", "Bearer fixture-machine", true)
        .add_header("x-hagency-generation", "1", true)
        .send(&f.service)
        .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::OK
    );
    let lease = response.take_json::<Value>().await.unwrap();
    assert_eq!(lease["delivery"]["id"], "t1");
    assert_eq!(
        lease["delivery"]["payload"]["body"]["events"][0]["content"]["a"],
        0.5
    );
    let ack = json!({"lane":"matrix","id":"t1","token":lease["delivery"]["token"]});
    let response = TestClient::post(format!("https://operations.test/api/fleet/v2/{id}/ack"))
        .add_header("host", "operations.test", true)
        .add_header("authorization", "Bearer fixture-machine", true)
        .add_header("x-hagency-generation", "1", true)
        .json(&ack)
        .send(&f.service)
        .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::OK
    );
    let mut response = TestClient::get(&url)
        .add_header("host", "operations.test", true)
        .add_header("authorization", "Bearer fixture-machine", true)
        .add_header("x-hagency-generation", "1", true)
        .send(&f.service)
        .await;
    assert!(response.take_json::<Value>().await.unwrap()["delivery"].is_null());
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["fleets"][&id]["state"],
        "pending_connection"
    );
    // A human mini-app bearer and a browser Origin confer no machine authority.
    let human = f.session("manager").await;
    let response = TestClient::get(&url)
        .add_header("host", "operations.test", true)
        .add_header("authorization", format!("Bearer {human}"), true)
        .add_header("x-hagency-generation", "1", true)
        .send(&f.service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
    let response = TestClient::get(&url)
        .add_header("host", "operations.test", true)
        .add_header("origin", "https://operations.test", true)
        .add_header("authorization", "Bearer fixture-machine", true)
        .add_header("x-hagency-generation", "1", true)
        .send(&f.service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
}

#[tokio::test]
async fn connection_verified_requires_exact_probe_original_matrix_ack_and_current_membership() {
    let f = Fixture::new().await;
    let fleet = format!("hf_{}", "c".repeat(32));
    let representative = format!("@{fleet}_representative:example.test");
    f.app.store.lock().await.transaction(|state|{
        state["fleets"][&fleet]=json!({"id":fleet,"state":"pending_connection","installation":"installed",
            "ownerMxid":"@provider:example.test","representativeMxid":representative,
            "transport":{"mode":"outbound","generation":1,"token":"fixture-machine","sequence":0},
            "registration":{"as_token":"fixture-as","hs_token":"fixture-relay"},
            "probe":{"roomId":"!reception:example.test","eventId":"$probe","challenge":"challenge_one"}});
        Ok(())
    }).unwrap();
    let event = json!({"type":"com.hagency.connection.probe.v1","sender":representative,"room_id":"!reception:example.test","event_id":"$probe",
        "content":{"fleetId":fleet,"challenge":"challenge_one"}});
    let response = TestClient::put(format!(
        "https://relay.test/api/relay/v2/{fleet}/transactions/probe_tx"
    ))
    .add_header("host", "relay.test", true)
    .add_header("authorization", "Bearer fixture-relay", true)
    .json(&json!({"events":[event]}))
    .send(&f.service)
    .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::OK
    );
    let update = json!({"v":2,"generation":1,"sequence":1,"heartbeat":true,"probeReceipts":[{
        "received":true,"fleetId":fleet,"sourceRoomId":"!reception:example.test","sourceEventId":"$probe","challenge":"challenge_one"}]});
    let (status, body) = f.machine(&fleet, "updates", update.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "matrix_receipt_pending");
    let mut response=TestClient::get(format!("https://operations.test/api/fleet/v2/{fleet}/poll?lane=matrix&consumer=01234567-0123-0123-0123-0123456789ab&wait=0"))
        .add_header("host","operations.test",true).add_header("authorization","Bearer fixture-machine",true).add_header("x-hagency-generation","1",true).send(&f.service).await;
    let lease = response.take_json::<Value>().await.unwrap();
    assert_eq!(
        f.machine(
            &fleet,
            "ack",
            json!({"lane":"matrix","id":"probe_tx","token":lease["delivery"]["token"]})
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut wrong = update.clone();
    wrong["probeReceipts"][0]["challenge"] = json!("different");
    assert_eq!(
        f.machine(&fleet, "updates", wrong).await.0,
        StatusCode::CONFLICT
    );
    let (status, body) = f.machine(&fleet, "updates", update.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let state = f.app.store.lock().await.read().unwrap();
    assert_eq!(state["fleets"][&fleet]["state"], "ready");
    assert_eq!(
        state["fleets"][&fleet]["connection"]["sourceEventId"],
        "$probe"
    );
    assert_eq!(state["fleets"][&fleet]["connection"]["generation"], 1);
    assert_eq!(f.machine(&fleet, "updates", update).await.0, StatusCode::OK);
}
