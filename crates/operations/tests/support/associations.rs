use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;

#[derive(Default)]
pub(super) struct Registrations {
    pub(super) values: BTreeMap<String, Value>,
    users: BTreeMap<String, Value>,
    lose_install_reply: bool,
    installs: usize,
}
fn path(segments: &[&str]) -> String {
    let mut url = reqwest::Url::parse("https://fixture.test").unwrap();
    url.path_segments_mut().unwrap().clear().extend(segments);
    url.path().into()
}
pub(super) async fn matrix(
    req: &mut salvo::Request,
    depot: &mut Depot,
    res: &mut Response,
) -> bool {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned();
    let path = req.uri().path().to_owned();
    let body = if req.method() == reqwest::Method::POST && path == "/_palpo/admin/v1/appservices" {
        req.parse_json::<Value>().await.unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let mut registrations = depot
        .get_typed::<Arc<Mutex<Registrations>>>()
        .unwrap()
        .lock()
        .unwrap();
    if path.ends_with("/whoami") {
        if let Some(fleet) = registrations
            .values
            .values()
            .find(|v| v["as_token"] == token)
            .cloned()
        {
            let user = req.query::<String>("user_id").unwrap_or_default();
            assert!(user.starts_with(&format!("@{}_", fleet["id"].as_str().unwrap())));
            registrations.users.insert(
                self::path(&["_palpo", "admin", "v2", "users", &user]),
                json!({"name":user,"appservice_id":fleet["id"],"deactivated":false,"locked":false}),
            );
            res.render(Json(json!({"user_id":user})));
            return true;
        }
        return false;
    }
    if token != "admin-token" {
        return false;
    }
    if path == "/_palpo/admin/v1/appservices" {
        if req.method() == reqwest::Method::POST {
            registrations.installs += 1;
            registrations
                .values
                .insert(body["id"].as_str().unwrap().into(), body);
            if std::mem::take(&mut registrations.lose_install_reply) {
                res.status_code(StatusCode::BAD_GATEWAY);
                res.render(Json(json!({"errcode":"M_UNKNOWN"})));
                return true;
            }
            res.render(Json(json!({})));
            return true;
        }
        res.render(Json(json!({"appservices":registrations.values.keys().map(|id|json!({"id":id})).collect::<Vec<_>>()})));
        return true;
    }
    if let Some(id) = path.strip_prefix("/_palpo/admin/v1/appservices/") {
        if let Some(value) = registrations.values.get(id) {
            res.render(Json(value.clone()));
        } else {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(json!({"errcode":"M_NOT_FOUND"})));
        }
        return true;
    }
    if path.starts_with("/_palpo/admin/v2/users/") {
        if let Some(value) = registrations.users.get(&path) {
            res.render(Json(value.clone()));
        } else if ["provider", "coordinator", "manager", "admin"]
            .iter()
            .any(|user| {
                self::path(&[
                    "_palpo",
                    "admin",
                    "v2",
                    "users",
                    &format!("@{user}:example.test"),
                ]) == path
            })
        {
            res.render(Json(
                json!({"deactivated":false,"locked":false,"appservice_id":null}),
            ));
        } else {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(json!({"errcode":"M_NOT_FOUND"})));
        }
        return true;
    }
    false
}
fn intent(request: &str, coordinator: &str) -> Value {
    json!({"requestId":request,"name":"Hagency installation","runtimeId":"d".repeat(64),"coordinatorMxid":format!("@{coordinator}:example.test"),
        "delegationExpiresAtMs":now_ms()+86400000,"exportMxids":[format!("@{coordinator}:example.test")]})
}

#[tokio::test]
async fn connection_retries_keep_one_probe_and_do_not_confuse_verification_with_online() {
    let f = Fixture::new().await;
    let (_, request) = f
        .post(
            "association-request",
            "provider-token",
            intent("probe_recovery", "coordinator"),
        )
        .await;
    let id = &request["action"]["id"];
    let fleet = request["action"]["fleetId"].as_str().unwrap();
    let admin = f.session("admin").await;
    let owner = f.session("provider").await;
    let coordinator = f.session("coordinator").await;
    let result = f
        .call(
            &admin,
            "palpo.inbox.decide",
            json!({"id":id,"decision":"approve","commandId":"approve_probe","expectedRevision":1}),
        )
        .await;
    assert_eq!(result.0, StatusCode::OK, "{result:?}");
    assert_eq!(
        f.call(&admin, "palpo.fleets.connect", json!({"fleetId":fleet}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.call(
            &coordinator,
            "palpo.fleets.connect",
            json!({"fleetId":fleet})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    f.rooms.lock().unwrap().lose_create_reply = true;
    assert_eq!(
        f.call(&owner, "palpo.fleets.connect", json!({"fleetId":fleet}))
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    f.rooms.lock().unwrap().lose_event_reply = true;
    assert_eq!(
        f.call(&owner, "palpo.fleets.connect", json!({"fleetId":fleet}))
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    let result = f
        .call(&owner, "palpo.fleets.connect", json!({"fleetId":fleet}))
        .await;
    assert_eq!(result.0, StatusCode::OK, "{result:?}");
    assert_eq!(result.1["fleet"]["connectionVerified"], false);
    assert_eq!(result.1["fleet"]["connectivity"], "offline");
    assert_eq!(
        f.call(&owner, "palpo.fleets.connect", json!({"fleetId":fleet}))
            .await
            .1,
        result.1
    );
    assert_eq!(f.rooms.lock().unwrap().create_count, 1);
    assert_eq!(f.rooms.lock().unwrap().events.len(), 1);
    let state = f.app.store.lock().await.read().unwrap();
    let record = &state["fleets"][fleet];
    let probe = &record["probe"];
    let machine = record["transport"]["token"].as_str().unwrap();
    let relay = record["registration"]["hs_token"].as_str().unwrap();
    let event = json!({"type":"com.hagency.connection.probe.v1","sender":record["representativeMxid"],"room_id":probe["roomId"],"event_id":probe["eventId"],
        "content":{"v":1,"fleetId":fleet,"challenge":probe["challenge"]}});
    let response = TestClient::put(format!(
        "https://relay.test/api/relay/v2/{fleet}/transactions/probe_one"
    ))
    .add_header("host", "relay.test", true)
    .add_header("authorization", format!("Bearer {relay}"), true)
    .json(&json!({"events":[event]}))
    .send(&f.service)
    .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::OK
    );
    let update = json!({"v":2,"generation":1,"sequence":1,"heartbeat":true,"probeReceipts":[{"received":true,"fleetId":fleet,
        "sourceRoomId":probe["roomId"],"sourceEventId":probe["eventId"],"challenge":probe["challenge"]}],
        "capabilities":{"v":1,"fleetId":fleet,"serverName":"example.test","representativeMxid":record["representativeMxid"],"approvalBotMxid":format!("@{fleet}_approval:example.test"),"offers":[],"coordinatorApprovalV1":true}});
    let mut lease=TestClient::get(format!("https://operations.test/api/fleet/v2/{fleet}/poll?lane=matrix&consumer=01234567-0123-0123-0123-0123456789ab&wait=0"))
        .add_header("host","operations.test",true).add_header("authorization",format!("Bearer {machine}"),true).add_header("x-hagency-generation","1",true).send(&f.service).await;
    let lease = lease.take_json::<Value>().await.unwrap();
    let ack = TestClient::post(format!("https://operations.test/api/fleet/v2/{fleet}/ack"))
        .add_header("host", "operations.test", true)
        .add_header("authorization", format!("Bearer {machine}"), true)
        .add_header("x-hagency-generation", "1", true)
        .json(&json!({"lane":"matrix","id":"probe_one","token":lease["delivery"]["token"]}))
        .send(&f.service)
        .await;
    assert_eq!(ack.status_code.unwrap_or(StatusCode::OK), StatusCode::OK);
    let mut response = TestClient::post(format!(
        "https://operations.test/api/fleet/v2/{fleet}/updates"
    ))
    .add_header("host", "operations.test", true)
    .add_header("authorization", format!("Bearer {machine}"), true)
    .add_header("x-hagency-generation", "1", true)
    .json(&update)
    .send(&f.service)
    .await;
    assert_eq!(
        response.status_code.unwrap_or(StatusCode::OK),
        StatusCode::OK,
        "{:?}",
        response.take_string().await
    );
    let list = f.call(&owner, "palpo.fleets.list", json!({})).await.1;
    let row = list["fleets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == fleet)
        .unwrap();
    assert_eq!(row["connectionVerified"], true);
    assert_eq!(row["connectivity"], "online");
    assert!(row["lastVerifiedAt"].is_string());
    assert!(!list.to_string().contains(machine));
    assert!(!list.to_string().contains(relay));
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            state["fleets"][fleet]["transport"]["lastSeenAt"] = json!("2020-01-01T00:00:00Z");
            Ok(())
        })
        .unwrap();
    let list = f.call(&owner, "palpo.fleets.list", json!({})).await.1;
    let row = list["fleets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == fleet)
        .unwrap();
    assert_eq!(row["connectionVerified"], true);
    assert_eq!(row["connectivity"], "offline");
    assert_eq!(
        f.call(&owner, "palpo.inbox.get", json!({"id":id})).await.1["action"]["execution"],
        "verified"
    );
}

#[tokio::test]
async fn association_has_one_admin_decision_and_recovers_install_before_scoped_export() {
    let f = Fixture::new().await;
    let input = intent("association_one", "coordinator");
    let (status, requested) = f
        .post("association-request", "provider-token", input.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{requested}");
    assert_eq!(
        f.post("association-request", "provider-token", input)
            .await
            .1,
        requested
    );
    let id = requested["action"]["id"].as_str().unwrap();
    let fleet = requested["action"]["fleetId"].as_str().unwrap();
    assert_eq!(requested["action"]["state"], "requested");
    let manager = f.session("manager").await;
    let provider = f.session("provider").await;
    let coordinator = f.session("coordinator").await;
    let admin = f.session("admin").await;
    assert_eq!(
        f.call(&manager, "palpo.inbox.get", json!({"id":id}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let decision = json!({"id":id,"decision":"approve","commandId":"association_approval","expectedRevision":1,"reason":"Approve this runtime connection"});
    assert_eq!(
        f.call(&coordinator, "palpo.inbox.decide", decision.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    f.registrations.lock().unwrap().lose_install_reply = true;
    let (status, approved) = f.call(&admin, "palpo.inbox.decide", decision.clone()).await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["action"]["state"], "approved");
    assert_eq!(approved["action"]["execution"], "setup_failed");
    let (status, retried) = f.call(&admin, "palpo.inbox.decide", decision).await;
    assert_eq!(status, StatusCode::OK, "{retried}");
    assert_eq!(retried["action"]["execution"], "verifying");
    assert_eq!(f.registrations.lock().unwrap().installs, 1);
    assert_eq!(
        f.call(&provider, "palpo.fleets.export", json!({"fleetId":fleet}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, export) = f
        .call(&admin, "palpo.fleets.export", json!({"fleetId":fleet}))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        f.call(
            &coordinator,
            "palpo.fleets.export",
            json!({"fleetId":fleet})
        )
        .await
        .1,
        export
    );
    assert_eq!(export["fleetId"], fleet);
    assert_eq!(export["runtimeId"], "d".repeat(64));
    assert_eq!(export["engagement"]["state"], "verifying");
    assert_eq!(export["engagement"]["coordinatorApprovalV1"], false);
    assert_ne!(
        export["registration"]["as_token"],
        export["transport"]["token"]
    );
    let serialized = retried.to_string();
    for secret in [
        &export["registration"]["as_token"],
        &export["registration"]["hs_token"],
        &export["transport"]["token"],
    ] {
        assert!(!serialized.contains(secret.as_str().unwrap()));
    }
    let state = f.app.store.lock().await.read().unwrap();
    let w = Workflows::load(&state).unwrap();
    assert!(
        !w.authority
            .resources
            .values()
            .any(|r| r.server_engagement_id.as_str() == fleet)
    );
    assert_eq!(
        w.receipts.values().filter(|r| r["actionId"] == id).count(),
        1
    );
}

#[tokio::test]
async fn two_same_server_associations_keep_distinct_profiles_and_owners_cannot_select_admin() {
    let f = Fixture::new().await;
    let admin = f.session("admin").await;
    let mut profiles = Vec::new();
    for (request, coordinator) in [("first", "coordinator"), ("second", "manager")] {
        let (_, row) = f
            .post(
                "association-request",
                "provider-token",
                intent(request, coordinator),
            )
            .await;
        let id = &row["action"]["id"];
        let fleet = &row["action"]["fleetId"];
        let result=f.call(&admin,"palpo.inbox.decide",json!({"id":id,"decision":"approve","expectedRevision":1,"commandId":format!("approve_{request}")})).await;
        assert_eq!(result.0, StatusCode::OK, "{:?}", result);
        let (status, profile) = f
            .call(&admin, "palpo.fleets.export", json!({"fleetId":fleet}))
            .await;
        assert_eq!(status, StatusCode::OK);
        profiles.push(profile);
    }
    assert_ne!(profiles[0]["fleetId"], profiles[1]["fleetId"]);
    assert_ne!(
        profiles[0]["transport"]["token"],
        profiles[1]["transport"]["token"]
    );
    assert_ne!(
        profiles[0]["registration"]["as_token"],
        profiles[1]["registration"]["as_token"]
    );
    assert_ne!(
        profiles[0]["engagement"]["coordinator"],
        profiles[1]["engagement"]["coordinator"]
    );
    assert_eq!(f.registrations.lock().unwrap().installs, 2);
    let mut forged = intent("forged", "coordinator");
    forged["administratorMxid"] = json!("@provider:example.test");
    assert_eq!(
        f.post("association-request", "provider-token", forged)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut forged = intent("arbitrary_export", "coordinator");
    forged["exportMxids"] = json!(["@manager:example.test"]);
    assert_eq!(
        f.post("association-request", "provider-token", forged)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}
