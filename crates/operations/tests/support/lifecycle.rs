use super::*;
use palpo_operations::digest;

#[tokio::test]
async fn scoped_agent_control_waits_for_cleanup_and_retries_only_definitive_failure() {
    let f = Fixture::new().await;
    let fleet = format!("hf_{}", "e".repeat(32));
    f.app.store.lock().await.transaction(|state| {
        let mut snapshot=authority(now_ms());
        let mut e=snapshot["engagements"]["engagement_a"].take();e["id"]=json!(fleet);
        snapshot["engagements"]=json!({&fleet:e});
        snapshot["resources"]["grant_a"]["serverEngagementId"]=json!(fleet);
        snapshot["projects"]["existing_project"]["serverEngagementId"]=json!(fleet);
        Workflows{authority:serde_json::from_value(snapshot)?,..Default::default()}.save(state)?;
        state["fleets"][&fleet]=json!({"id":fleet,"registrationGeneration":1,"state":"ready","installation":"installed",
            "representativeMxid":format!("@{fleet}_representative:example.test"),
            "transport":{"mode":"outbound","generation":1,"token":"fixture-machine"},"capabilities":{"coordinatorApprovalV1":true,"coordinatorAgentControlV1":true}});Ok(())
    }).unwrap();
    let owner = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let admin = f.session("admin").await;
    let mut request = agent_request();
    request["request"]["serverEngagementId"] = json!(fleet);
    let definition = json!({"v":1,"fleetId":fleet,"requestId":"request_agent","targetProjectId":"existing_project","agentDefinition":{"name":"Managed"}});
    request["request"]["definitionDigest"] = json!(digest(&definition).unwrap());
    request["definition"] = definition.clone();
    let (code, submitted) = f.call(&owner, "palpo.inbox.submit", request.clone()).await;
    assert_eq!(code, StatusCode::OK, "{submitted}");
    let id = submitted["action"]["id"].as_str().unwrap();
    let mut approve = approval(&request, "allocation");
    approve["context"]["serverEngagementId"] = json!(fleet);
    let (code, result) = f
        .call(
            &coordinator,
            "palpo.inbox.decide",
            json!({"id":id,"decision":"approve","command":approve}),
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{result}");
    let status = json!({"v":1,"fleetId":fleet,"requestId":"request_agent","engagementId":"agent_one","state":"active",
        "targetProjectId":"existing_project","agentDefinition":{"name":"Managed"},"allocatedTokens":100000,
        "bound":false,"ready":false,"observedAt":"2026-10-04T00:00:00Z","lifecycle":{"runtimeState":"active","paused":false,"cleanup":"not_required","cleanupEffect":null}});
    assert_eq!(
        f.machine(
            &fleet,
            "updates",
            json!({"v":2,"generation":1,"sequence":1,"heartbeat":true,"statuses":[status]})
        )
        .await
        .0,
        StatusCode::OK
    );
    let control = json!({"kind":"agent_control","agentActionId":id,"commandId":"remove_agent","operation":"retire"});
    assert_eq!(
        f.call(&admin, "palpo.agents.control", control.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (code, result) = f
        .call(&owner, "palpo.agents.control", control.clone())
        .await;
    assert_eq!(code, StatusCode::OK, "{result}");
    assert_eq!(
        result["action"]["agentControl"]["execution"],
        "control_pending"
    );
    assert_eq!(
        f.call(&owner, "palpo.agents.control", control.clone())
            .await
            .0,
        StatusCode::OK
    );
    let record =
        f.app.store.lock().await.read().unwrap()["rustWorkflows"]["agentControls"]["remove_agent"]
            .clone();
    let receipt = json!({"kind":"receipt","registrationGeneration":1,"delegationRevision":1,"commandId":"remove_agent",
        "commandDigest":record["commandDigest"],"agentId":"agent_one","operation":"retire","state":"applied"});
    let mut invalid_refusal = receipt.clone();
    invalid_refusal["state"] = json!("refused");
    invalid_refusal["reason"] = json!("unbounded_provider_error");
    assert_eq!(f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":2,"heartbeat":true,
        "coordinatorUpdates":[{"id":"command_remove_agent","payload":invalid_refusal,"digest":digest(&invalid_refusal).unwrap()}]})).await.0, StatusCode::CONFLICT);
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["rustWorkflows"]["agentControls"]["remove_agent"]
            ["state"],
        "pending"
    );
    let (code,result)=f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":2,"heartbeat":true,
        "coordinatorUpdates":[{"id":"command_remove_agent","payload":receipt,"digest":digest(&receipt).unwrap()}]})).await;
    assert_eq!(code, StatusCode::OK, "{result}");
    assert_eq!(
        f.call(&owner, "palpo.inbox.get", json!({"id":id})).await.1["action"]["agentControl"]["execution"],
        "retiring"
    );
    let retry = json!({"agentActionId":id,"commandId":"retry_remove","operation":"retry_cleanup"});
    assert_eq!(
        f.call(&owner, "palpo.agents.control", retry.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut final_status = status.clone();
    final_status["state"] = json!("ended");
    final_status["lifecycle"] = json!({"runtimeState":"revoked","paused":false,"cleanup":"uncertain","cleanupEffect":"uncertain"});
    assert_eq!(
        f.machine(
            &fleet,
            "updates",
            json!({"v":2,"generation":1,"sequence":3,"heartbeat":true,"statuses":[final_status]})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(&owner, "palpo.agents.control", retry.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    final_status["lifecycle"]["cleanup"] = json!("pending");
    final_status["lifecycle"]["cleanupEffect"] = json!("failed");
    assert_eq!(
        f.machine(
            &fleet,
            "updates",
            json!({"v":2,"generation":1,"sequence":4,"heartbeat":true,"statuses":[final_status]})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(&owner, "palpo.requests.list", json!({})).await.1["requests"][0]["canRetryCleanup"],
        true
    );
    assert_eq!(
        f.call(&owner, "palpo.agents.control", retry).await.0,
        StatusCode::OK
    );
    let retry_record =
        f.app.store.lock().await.read().unwrap()["rustWorkflows"]["agentControls"]["retry_remove"]
            .clone();
    let mut retry_receipt = receipt;
    retry_receipt["commandId"] = json!("retry_remove");
    retry_receipt["operation"] = json!("retry_cleanup");
    retry_receipt["commandDigest"] = retry_record["commandDigest"].clone();
    final_status["lifecycle"]["cleanup"] = json!("complete");
    final_status["lifecycle"]["cleanupEffect"] = json!("complete");
    let (code,result)=f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":5,"heartbeat":true,"statuses":[final_status],
        "coordinatorUpdates":[{"id":"command_retry_remove","payload":retry_receipt,"digest":digest(&retry_receipt).unwrap()}]})).await;
    assert_eq!(code, StatusCode::OK, "{result}");
    let row = f.call(&owner, "palpo.requests.list", json!({})).await.1;
    assert_eq!(row["requests"][0]["execution"], "retired");
    assert_eq!(row["requests"][0]["usable"], false);
    assert_eq!(row["requests"][0]["usage"]["consumedTokens"], Value::Null);
    assert_eq!(
        f.call(&owner, "palpo.agents.control", control).await.0,
        StatusCode::OK
    );
}
