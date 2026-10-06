use super::*;
const FLEET: &str = "hf_0123456789abcdef0123456789abcdef";
const MXID: &str =
    "@hf_0123456789abcdef0123456789abcdef_en_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:example.test";
fn user_path() -> String {
    path_for(MXID)
}
fn path_for(mxid: &str) -> String {
    let mut url = reqwest::Url::parse("https://fixture.test").unwrap();
    url.path_segments_mut()
        .unwrap()
        .clear()
        .extend(["_palpo", "admin", "v2", "users", mxid]);
    url.path().into()
}
async fn fixture() -> Fixture {
    let f = Fixture::new().await;
    f.app.store.lock().await.transaction(|s| {
        s["fleets"][FLEET]=json!({"id":FLEET,"state":"ready","installation":"installed","registration":{"as_token":"retirement-appservice"},"transport":{"mode":"outbound","token":"retirement-machine","generation":3}});
        s["requests"][format!("{FLEET}:original")]=json!({"fleetId":FLEET,"requestId":"original","state":"active","provider":{"agentMxid":MXID}});Ok(())
    }).unwrap();
    let mut registrations = f.registrations.lock().unwrap();
    registrations.values.insert(
        FLEET.into(),
        json!({"id":FLEET,"as_token":"retirement-appservice"}),
    );
    registrations.users.insert(
        user_path(),
        json!({"appservice_id":FLEET,"admin":false,"deactivated":false}),
    );
    drop(registrations);
    f
}
async fn retire(f: &Fixture, input: Value, generation: u64) -> (StatusCode, Value) {
    let mut response = TestClient::post(format!(
        "http://operations.test/api/fleet/v2/{FLEET}/retire-agent"
    ))
    .add_header("host", "operations.test", true)
    .add_header("authorization", "Bearer retirement-machine", true)
    .add_header("x-hagency-generation", generation.to_string(), true)
    .json(&input)
    .send(&f.service)
    .await;
    (
        response.status_code.unwrap_or(StatusCode::OK),
        response.take_json().await.unwrap(),
    )
}
fn input() -> Value {
    json!({"requestId":"original","agentMxid":MXID})
}

#[tokio::test]
async fn retirement_of_partial_native_provision_uses_only_the_approved_deterministic_identity() {
    let f = fixture().await;
    let request = "partial_provision";
    let mxid = format!(
        "@{FLEET}_en_{}:example.test",
        &palpo_operations::digest(&json!([FLEET, request])).unwrap()[..32]
    );
    f.app.store.lock().await.transaction(|s| {
        let mut workflows=Workflows::load(s)?;
        let mut r=agent_request();r["request"]["id"]=json!(request);r["request"]["serverEngagementId"]=json!(FLEET);
        workflows.actions.insert("partial_action".into(),serde_json::from_value(json!({"id":"partial_action","request":r,"state":"approved","revision":2,"createdAt":1,"updatedAt":2,"decision":{},"execution":"provisioning"}))?);
        workflows.save(s)
    }).unwrap();
    f.registrations.lock().unwrap().users.insert(
        path_for(&mxid),
        json!({"appservice_id":FLEET,"admin":false,"deactivated":false}),
    );
    let wrong = retire(&f, json!({"requestId":request,"agentMxid":MXID}), 3).await;
    assert_eq!(wrong.0, StatusCode::FORBIDDEN, "{wrong:?}");
    let result = retire(&f, json!({"requestId":request,"agentMxid":mxid}), 3).await;
    assert_eq!(result.0, StatusCode::OK, "{result:?}");
    assert_eq!(result.1["agent"]["mxid"], mxid);
}
#[tokio::test]
async fn retirement_recovers_lost_deactivation_and_rechecks_a_completed_identity() {
    let f = fixture().await;
    f.registrations.lock().unwrap().lose_retirement_reply = true;
    let failed = retire(&f, input(), 3).await;
    assert_eq!(failed.0, StatusCode::BAD_GATEWAY, "{failed:?}");
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["identityRetirements"]
            [format!("{FLEET}:original")]["state"],
        "incomplete"
    );
    let recovered = retire(&f, input(), 3).await;
    assert_eq!(recovered.0, StatusCode::OK, "{recovered:?}");
    assert_eq!(recovered.1["agent"]["appserviceAccess"], "revoked");
    assert_eq!(recovered.1["agent"]["localTaskStop"], "unconfirmed");
    assert_eq!(f.registrations.lock().unwrap().retirement_calls, 1);
    assert_eq!(retire(&f, input(), 3).await.1, recovered.1);
    assert_eq!(f.registrations.lock().unwrap().retirement_calls, 1);
    f.registrations
        .lock()
        .unwrap()
        .users
        .get_mut(&user_path())
        .unwrap()["deactivated"] = json!(false);
    assert_eq!(retire(&f, input(), 3).await.0, StatusCode::OK);
    assert_eq!(f.registrations.lock().unwrap().retirement_calls, 2);
    let history = f.app.store.lock().await.read().unwrap();
    assert_eq!(
        history["requests"][format!("{FLEET}:original")]["provider"]["agentMxid"],
        MXID
    );
    assert!(!recovered.1.to_string().contains("retirement-appservice"));
}
#[tokio::test]
async fn retirement_requires_exact_scope_and_no_other_live_allocation() {
    let f = fixture().await;
    assert_eq!(retire(&f, input(), 2).await.0, StatusCode::CONFLICT);
    assert_eq!(
        retire(&f, json!({"requestId":"other","agentMxid":MXID}), 3)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(retire(&f,json!({"requestId":"original","agentMxid":format!("@{FLEET}_representative:example.test")}),3).await.0,StatusCode::FORBIDDEN);
    f.app
        .store
        .lock()
        .await
        .transaction(|s| {
            s["requests"]["other"] =
                json!({"fleetId":FLEET,"state":"active","provider":{"agentMxid":MXID}});
            Ok(())
        })
        .unwrap();
    assert_eq!(retire(&f, input(), 3).await.0, StatusCode::CONFLICT);
    assert_eq!(f.registrations.lock().unwrap().retirement_calls, 0);
    f.app
        .store
        .lock()
        .await
        .transaction(|s| {
            s["requests"]["other"]["state"] = json!("ended");
            s["fleets"][FLEET]["state"] = json!("revoked");
            Ok(())
        })
        .unwrap();
    assert_eq!(
        retire(&f, input(), 3).await.0,
        StatusCode::OK,
        "revoked credentials may only finish their own cleanup"
    );
}
#[tokio::test]
async fn retirement_does_not_complete_without_room_removal_and_appservice_denial() {
    let f = fixture().await;
    f.registrations
        .lock()
        .unwrap()
        .retirement_rooms
        .push("!stilljoined:example.test".into());
    assert_eq!(retire(&f, input(), 3).await.0, StatusCode::BAD_GATEWAY);
    {
        let mut r = f.registrations.lock().unwrap();
        r.retirement_rooms.clear();
        r.allow_retired_auth = true;
    }
    assert_eq!(retire(&f, input(), 3).await.0, StatusCode::BAD_GATEWAY);
    f.registrations.lock().unwrap().allow_retired_auth = false;
    assert_eq!(retire(&f, input(), 3).await.0, StatusCode::OK);
}
