use palpo_operations::digest;

use super::*;

#[tokio::test]
async fn authenticated_refusal_keeps_the_approval_but_ends_execution_and_survives_replays() {
    let f = Fixture::new().await;
    let fleet = format!("hf_{}", "e".repeat(32));
    f.app.store.lock().await.transaction(|state|{
        let mut snapshot=authority(now_ms());
        let mut engagement=snapshot["engagements"]["engagement_a"].take();engagement["id"]=json!(fleet);
        snapshot["engagements"]=json!({&fleet:engagement});
        snapshot["resources"]["grant_a"]["serverEngagementId"]=json!(fleet);
        snapshot["projects"]["existing_project"]["serverEngagementId"]=json!(fleet);
        Workflows{authority:serde_json::from_value(snapshot)?,..Default::default()}.save(state)?;
        state["fleets"][&fleet]=json!({"id":fleet,"registrationGeneration":1,"state":"ready","installation":"installed",
            "transport":{"mode":"outbound","generation":1,"token":"fixture-machine"},"capabilities":{"coordinatorApprovalV1":true}});Ok(())
    }).unwrap();
    let definition = json!({"v":1,"fleetId":fleet,"requestId":"request_agent","targetProjectId":"existing_project","agentDefinition":{"name":"VisibleRefusal"}});
    let mut request = agent_request();
    request["request"]["serverEngagementId"] = json!(fleet);
    request["request"]["definitionDigest"] = json!(digest(&definition).unwrap());
    request["definition"] = definition;
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let (status, submitted) = f
        .call(&manager, "palpo.inbox.submit", request.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    let id = &submitted["action"]["id"];
    let mut command = approval(&request, "refusal_decision");
    command["context"]["serverEngagementId"] = json!(fleet);
    let decision = json!({"id":id,"decision":"approve","command":command});
    let (status, result) = f
        .call(&coordinator, "palpo.inbox.decide", decision.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let receipt = json!({"kind":"receipt","commandId":"refusal_decision","commandDigest":digest(&json!({"operation":"coordinator_agent_approval","command":command})).unwrap(),
        "state":"refused","reason":"insufficient_capacity","registrationGeneration":1,"delegationRevision":1});
    let update = json!({"v":2,"generation":1,"sequence":1,"heartbeat":true,"coordinatorUpdates":[{"id":"command_refusal_decision","payload":receipt,"digest":digest(&receipt).unwrap()}]});
    let (status, result) = f.machine(&fleet, "updates", update.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(f.machine(&fleet, "updates", update).await.0, StatusCode::OK);
    let row = f
        .call(&manager, "palpo.inbox.get", json!({"id":id}))
        .await
        .1;
    assert_eq!(row["action"]["state"], "approved");
    assert_eq!(row["action"]["execution"], "allocation_refused");
    assert_eq!(row["action"]["failureReason"], "insufficient_capacity");
    let row = f.call(&manager, "palpo.requests.list", json!({})).await.1;
    assert_eq!(row["requests"][0]["failureReason"], "insufficient_capacity");
    assert_eq!(row["requests"][0]["canRequestTopUp"], false);
    assert_eq!(
        f.call(&coordinator, "palpo.inbox.decide", decision).await.1["action"]["execution"],
        "allocation_refused"
    );
    let mut changed = receipt;
    changed["state"] = json!("applied");
    changed["agentId"] = json!("invented_agent");
    let (status,_)=f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":2,"heartbeat":true,"coordinatorUpdates":[{"id":"command_refusal_decision","payload":changed,"digest":digest(&changed).unwrap()}]})).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn owner_delegation_publication_replaces_reviewer_and_refuses_stale_or_foreign_authority() {
    let f = Fixture::new().await;
    let fleet = format!("hf_{}", "e".repeat(32));
    f.app.store.lock().await.transaction(|state| {
        let mut snapshot=authority(now_ms());
        let mut engagement=snapshot["engagements"]["engagement_a"].take();engagement["id"]=json!(fleet);
        snapshot["engagements"]=json!({&fleet:engagement});
        snapshot["resources"]["grant_a"]["serverEngagementId"]=json!(fleet);
        snapshot["projects"]["existing_project"]["serverEngagementId"]=json!(fleet);
        Workflows{authority:serde_json::from_value(snapshot)?,..Default::default()}.save(state)?;
        state["fleets"][&fleet]=json!({"id":fleet,"registrationGeneration":1,"state":"ready","installation":"installed",
            "transport":{"mode":"outbound","generation":1,"token":"fixture-machine"},"capabilities":{"coordinatorApprovalV1":true}});Ok(())
    }).unwrap();
    let manager = f.session("manager").await;
    let old_coordinator = f.session("coordinator").await;
    let new_coordinator = f.session("admin").await;
    let mut request = agent_request();
    request["request"]["serverEngagementId"] = json!(fleet);
    let (code, submitted) = f
        .call(&manager, "palpo.inbox.submit", request.clone())
        .await;
    assert_eq!(code, StatusCode::OK, "{submitted}");
    let id = &submitted["action"]["id"];
    assert_eq!(
        f.call(&old_coordinator, "palpo.inbox.get", json!({"id":id}))
            .await
            .1["action"]["canDecide"],
        true
    );
    assert_eq!(
        f.call(&new_coordinator, "palpo.inbox.get", json!({"id":id}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let state = f.app.store.lock().await.read().unwrap();
    let mut next = state["rustWorkflows"]["authority"]["engagements"][&fleet].clone();
    next["coordinator"] = json!("@admin:example.test");
    next["delegationRevision"] = json!(2);
    let publication = |value: Value| {
        let body = json!({"kind":"engagement","registrationGeneration":1,"delegationRevision":value["delegationRevision"],"engagement":value,"exportMxids":["@admin:example.test"],"observedAtMs":now_ms()});
        json!({"id":"engagement_authority","digest":digest(&body).unwrap(),"payload":body})
    };
    let update = json!({"v":2,"generation":1,"sequence":1,"heartbeat":true,"coordinatorUpdates":[publication(next.clone())]});
    let (code, result) = f.machine(&fleet, "updates", update.clone()).await;
    assert_eq!(code, StatusCode::OK, "{result}");
    assert_eq!(f.machine(&fleet, "updates", update).await.0, StatusCode::OK);
    assert_eq!(
        f.call(&old_coordinator, "palpo.inbox.get", json!({"id":id}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.call(&new_coordinator, "palpo.inbox.get", json!({"id":id}))
            .await
            .1["action"]["canDecide"],
        true
    );
    let stored = f.app.store.lock().await.read().unwrap();
    assert_eq!(
        stored["rustWorkflows"]["engagementExports"][&fleet],
        json!(["@admin:example.test"])
    );
    assert_eq!(
        stored["rustWorkflows"]["authority"]["resources"],
        state["rustWorkflows"]["authority"]["resources"]
    );
    let mut wrong = next.clone();
    wrong["owner"] = json!("@admin:example.test");
    wrong["delegationRevision"] = json!(3);
    assert_eq!(f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":2,"heartbeat":true,"coordinatorUpdates":[publication(wrong)]})).await.0,StatusCode::CONFLICT);
    next["state"] = json!("revoked");
    next["delegationRevision"] = json!(3);
    assert_eq!(f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":2,"heartbeat":true,"coordinatorUpdates":[publication(next.clone())]})).await.0,StatusCode::OK);
    assert_eq!(
        f.call(&new_coordinator, "palpo.inbox.get", json!({"id":id}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    next["state"] = json!("verified");
    next["delegationRevision"] = json!(4);
    assert_eq!(f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":3,"heartbeat":true,"coordinatorUpdates":[publication(next)]})).await.0,StatusCode::CONFLICT);
}
