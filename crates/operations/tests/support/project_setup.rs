use palpo_operations::digest;

use super::*;

#[tokio::test]
async fn project_setup_recovery_keeps_approval_and_replays_one_scoped_command() {
    let f = Fixture::new().await;
    let fleet = format!("hf_{}", "f".repeat(32));
    f.app.store.lock().await.transaction(|state|{
        let mut a=authority(now_ms());let mut e=a["engagements"]["engagement_a"].take();e["id"]=json!(fleet);a["engagements"]=json!({&fleet:e});a["resources"]["grant_a"]["serverEngagementId"]=json!(fleet);a["projects"]=json!({});
        Workflows{authority:serde_json::from_value(a)?,..Default::default()}.save(state)?;
        state["fleets"][&fleet]=json!({"id":fleet,"registrationGeneration":1,"state":"ready","installation":"installed","representativeMxid":format!("@{fleet}_representative:example.test"),"transport":{"mode":"outbound","generation":1,"token":"fixture-machine"},"capabilities":{"coordinatorApprovalV1":true,"coordinatorProjectSetupV1":true}});Ok(())
    }).unwrap();
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let admin = f.session("admin").await;
    let mut request = project_request();
    request["request"]["serverEngagementId"] = json!(fleet);
    let definition = json!({"name":"Recoverable","roomId":"!project:example.test","ownerDmRoomId":"!private:example.test"});
    request["request"]["definitionDigest"] = json!(digest(&definition).unwrap());
    request["definition"] = definition.clone();
    let (code, submitted) = f
        .call(&manager, "palpo.inbox.submit", request.clone())
        .await;
    assert_eq!(code, StatusCode::OK, "{submitted}");
    let id = submitted["action"]["id"].as_str().unwrap();
    let mut decision = approval(&request, "original_project_approval");
    decision["context"]["serverEngagementId"] = json!(fleet);
    let (code, result) = f
        .call(
            &coordinator,
            "palpo.inbox.decide",
            json!({"id":id,"decision":"approve","command":decision}),
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{result}");
    let grant = json!({"projectId":"new_project","serverEngagementId":fleet,"revision":1,"owner":"@manager:example.test","resourceAllocations":["grant_a"],"state":"approved"});
    let project =
        json!({"kind":"project","registrationGeneration":1,"delegationRevision":1,"project":grant});
    let receipt = json!({"kind":"receipt","registrationGeneration":1,"delegationRevision":1,"commandId":"original_project_approval","commandDigest":digest(&json!({"operation":"coordinator_project_approval","command":decision,"definition":definition})).unwrap(),"projectId":"new_project","state":"applied"});
    let mut status = json!({"kind":"project_setup","registrationGeneration":1,"delegationRevision":1,"projectId":"new_project","projectRevision":1,"approvalCommandId":"original_project_approval","attemptId":"original_project_approval","revision":1,"state":"failed","reason":"private_membership_pending","observedAtMs":now_ms()});
    let update =
        |id: &str, body: &Value| json!({"id":id,"payload":body,"digest":digest(body).unwrap()});
    let (code,result)=f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":1,"heartbeat":true,"coordinatorUpdates":[update("project_new_project",&project),update("command_original_project_approval",&receipt),update("project_setup_new_project",&status)]})).await;
    assert_eq!(code, StatusCode::OK, "{result}");
    let view = f
        .call(&manager, "palpo.inbox.get", json!({"id":id}))
        .await
        .1["action"]
        .clone();
    assert_eq!(view["execution"], "setup_failed");
    assert_eq!(view["canContinue"], true);
    let retry = json!({"id":id,"commandId":"retry_setup","expectedRevision":view["revision"]});
    assert_eq!(
        f.call(&admin, "palpo.inbox.activate", retry.clone())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (code, result) = f
        .call(&manager, "palpo.inbox.activate", retry.clone())
        .await;
    assert_eq!(code, StatusCode::OK, "{result}");
    assert_eq!(result["action"]["canContinue"], false);
    assert_eq!(
        f.call(&manager, "palpo.inbox.activate", retry.clone())
            .await
            .0,
        StatusCode::OK
    );
    let stored = f.app.store.lock().await.read().unwrap();
    assert_eq!(
        stored["rustWorkflows"]["outbox"].as_object().unwrap().len(),
        1
    );
    assert_eq!(
        stored["rustWorkflows"]["projectRetries"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        stored["rustWorkflows"]["projectRetries"]["retry_setup"]["command"]["approvalCommandId"],
        "original_project_approval"
    );
    let mut conflict = retry.clone();
    conflict["expectedRevision"] = json!(0);
    assert_eq!(
        f.call(&manager, "palpo.inbox.activate", conflict).await.0,
        StatusCode::CONFLICT
    );
    status["attemptId"] = json!("retry_setup");
    status["revision"] = json!(2);
    status["state"] = json!("ready");
    status["reason"] = Value::Null;
    let mut ready_project = project.clone();
    ready_project["project"]["state"] = json!("ready");
    let (code,result)=f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":2,"heartbeat":true,"coordinatorUpdates":[update("project_new_project",&ready_project),update("project_setup_new_project",&status)]})).await;
    assert_eq!(code, StatusCode::OK, "{result}");
    let view = f
        .call(&manager, "palpo.inbox.get", json!({"id":id}))
        .await
        .1["action"]
        .clone();
    assert_eq!(view["execution"], "ready");
    assert_eq!(view["canContinue"], false);
    assert_eq!(view["decision"]["commandId"], "original_project_approval");
    assert_eq!(
        f.call(&manager, "palpo.inbox.activate", retry).await.0,
        StatusCode::OK
    );
    status["revision"] = json!(3);
    status["attemptId"] = json!("unrequested_retry");
    assert_eq!(f.machine(&fleet,"updates",json!({"v":2,"generation":1,"sequence":3,"heartbeat":true,"coordinatorUpdates":[update("project_setup_new_project",&status)]})).await.0,StatusCode::CONFLICT);
}
