use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;

#[derive(Default)]
pub(super) struct Registrations {
    pub(super) values: BTreeMap<String, Value>,
    pub(super) users: BTreeMap<String, Value>,
    pub(super) lose_retirement_reply: bool,
    pub(super) retirement_calls: usize,
    pub(super) retirement_rooms: Vec<String>,
    pub(super) allow_retired_auth: bool,
    lose_install_reply: bool,
    installs: usize,
    pub(super) lose_control_reply: bool,
    pub(super) lose_url_reply: bool,
    pub(super) control_calls: usize,
    pub(super) url_calls: usize,
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
    let body = if matches!(*req.method(), reqwest::Method::POST | reqwest::Method::PUT)
        && path.starts_with("/_palpo/admin/v1/appservices")
    {
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
            if !registrations.allow_retired_auth
                && registrations
                    .users
                    .get(&self::path(&["_palpo", "admin", "v2", "users", &user]))
                    .is_some_and(|u| u["deactivated"] == true)
            {
                res.status_code(StatusCode::UNAUTHORIZED);
                res.render(Json(json!({"errcode":"M_UNKNOWN_TOKEN"})));
                return true;
            }
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
    if let Some(encoded) = path.strip_prefix("/_palpo/admin/v1/deactivate/") {
        let user_path = format!("/_palpo/admin/v2/users/{encoded}");
        if let Some(user) = registrations.users.get_mut(&user_path) {
            user["deactivated"] = json!(true);
        } else {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(json!({})));
            return true;
        }
        registrations.retirement_calls += 1;
        if std::mem::take(&mut registrations.lose_retirement_reply) {
            res.status_code(StatusCode::BAD_GATEWAY);
        }
        res.render(Json(json!({})));
        return true;
    }
    if path.starts_with("/_palpo/admin/v1/users/") && path.ends_with("/joined_rooms") {
        res.render(Json(json!({"joined_rooms":registrations.retirement_rooms})));
        return true;
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
        if let Some((id, operation)) = id.split_once('/') {
            if !registrations.values.contains_key(id) {
                res.status_code(StatusCode::NOT_FOUND);
                res.render(Json(json!({})));
                return true;
            }
            match operation {
                "disable" | "enable" => {
                    registrations.control_calls += 1;
                    registrations.values.get_mut(id).unwrap()["disabled"] =
                        json!(operation == "disable");
                    if std::mem::take(&mut registrations.lose_control_reply) {
                        res.status_code(StatusCode::BAD_GATEWAY);
                    }
                }
                "url" => {
                    assert_eq!(registrations.values[id]["url"], body["expected_url"]);
                    registrations.url_calls += 1;
                    registrations.values.get_mut(id).unwrap()["url"] = body["url"].clone();
                    if std::mem::take(&mut registrations.lose_url_reply) {
                        res.status_code(StatusCode::BAD_GATEWAY);
                    }
                }
                _ => panic!("unexpected appservice mutation"),
            }
            res.render(Json(json!({})));
            return true;
        }
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
    let action = f.call(&owner, "palpo.inbox.get", json!({"id":id})).await.1;
    assert_eq!(action["action"]["connectionVerification"], "unverified");
    assert_eq!(action["action"]["canConnect"], true);
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
    assert_eq!(result.1["fleet"]["connectionVerification"], "verifying");
    assert_eq!(
        f.call(&owner, "palpo.inbox.get", json!({"id":id})).await.1["action"]["connectionVerification"],
        "verifying"
    );
    assert_eq!(result.1["fleet"]["connectivity"], "offline");
    assert_eq!(
        f.call(&owner, "palpo.fleets.connect", json!({"fleetId":fleet}))
            .await
            .1,
        result.1
    );
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            state["fleets"][fleet]["probe"]["lastRequestedAtMs"] = json!(now_ms() - 120_001);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        f.call(&owner, "palpo.inbox.get", json!({"id":id})).await.1["action"]["connectionVerification"],
        "retry"
    );
    assert_eq!(
        f.call(&owner, "palpo.fleets.connect", json!({"fleetId":fleet}))
            .await
            .1["fleet"]["connectionVerification"],
        "verifying"
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
    assert_eq!(row["connectionVerification"], "verified");
    assert_eq!(
        f.call(&owner, "palpo.inbox.get", json!({"id":id})).await.1["action"]["connectionVerification"],
        "verified"
    );
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

#[tokio::test]
async fn legacy_connection_upgrade_keeps_namespace_credentials_agents_and_requires_new_probe() {
    let f = Fixture::new().await;
    let admin = f.session("admin").await;
    let (_, original) = f
        .post(
            "association-request",
            "provider-token",
            intent("legacy_seed", "coordinator"),
        )
        .await;
    let fleet = original["action"]["fleetId"].as_str().unwrap().to_owned();
    let result=f.call(&admin,"palpo.inbox.decide",json!({"id":original["action"]["id"],"decision":"approve","commandId":"legacy_install","expectedRevision":1})).await;
    assert_eq!(result.0, StatusCode::OK, "{result:?}");
    f.app
        .store
        .lock()
        .await
        .transaction(|s| {
            let mut w = Workflows::load(s)?;
            w.associations
                .remove(original["action"]["id"].as_str().unwrap());
            w.authority.engagements.remove(&fleet);
            w.save(s)?;
            let legacy = &mut s["fleets"][&fleet];
            legacy.as_object_mut().unwrap().remove("associationId");
            legacy.as_object_mut().unwrap().remove("runtimeId");
            legacy["state"] = json!("ready");
            legacy["connection"] = json!({"verifiedAt":"2026-01-01T00:00:00Z","generation":1});
            legacy["agents"] =
                json!({"original_agent":{"state":"approved","engagementId":"original_native_id"}});
            Ok(())
        })
        .unwrap();
    let before = f.app.store.lock().await.read().unwrap()["fleets"][&fleet].clone();
    let mut input = intent("upgrade_legacy", "coordinator");
    input["existingFleetId"] = json!(fleet);
    assert_eq!(
        f.post("association-request", "manager-token", input.clone())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, requested) = f
        .post("association-request", "provider-token", input.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{requested}");
    assert_eq!(requested["action"]["fleetId"], fleet);
    let manager = f.session("manager").await;
    let decision = json!({"id":requested["action"]["id"],"decision":"approve","commandId":"upgrade_approval","expectedRevision":1});
    assert_eq!(
        f.call(&manager, "palpo.inbox.decide", decision.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let result = f.call(&admin, "palpo.inbox.decide", decision.clone()).await;
    assert_eq!(result.0, StatusCode::OK, "{result:?}");
    assert_eq!(result.1["action"]["execution"], "verifying");
    assert_eq!(
        f.call(&admin, "palpo.inbox.decide", decision).await.0,
        StatusCode::OK
    );
    let replay = f
        .post("association-request", "provider-token", input.clone())
        .await;
    assert_eq!(replay.0, StatusCode::OK, "{replay:?}");
    assert_eq!(replay.1["action"]["id"], requested["action"]["id"]);
    let after = f.app.store.lock().await.read().unwrap();
    let legacy = &after["fleets"][&fleet];
    for key in [
        "registration",
        "transport",
        "agents",
        "representativeMxid",
        "ownerMxid",
    ] {
        assert_eq!(legacy[key], before[key], "{key}");
    }
    assert!(legacy["connection"].is_null());
    assert!(legacy["probe"].is_null());
    assert_eq!(
        after["rustWorkflows"]["authority"]["engagements"][&fleet]["state"],
        "verifying"
    );
    assert_eq!(
        f.registrations.lock().unwrap().installs,
        1,
        "Upgrade must not register another namespace"
    );
    input["requestId"] = json!("second_upgrade");
    assert_eq!(
        f.post("association-request", "provider-token", input)
            .await
            .0,
        StatusCode::CONFLICT
    );
}
