use super::*;

fn start_body(id: &str) -> Value {
    json!({"ownerMxid":"@provider:example.test","intent":{"requestId":id,"name":"My desktop Hagency",
        "runtimeId":"d".repeat(64),"coordinatorMxid":"@coordinator:example.test",
        "delegationExpiresAtMs":now_ms()+86400000,"exportMxids":[]}})
}
#[tokio::test]
async fn native_pairing_requires_owner_then_admin_and_delivers_only_to_bound_runtime() {
    let f = Fixture::new().await;
    let token = "a".repeat(64);
    let body = start_body("desktop_pair");
    let (status, begun) = f.post("association-start", &token, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{begun}");
    assert_eq!(
        f.post("association-start", &token, body.clone()).await.1,
        begun
    );
    assert_eq!(
        f.post("association-start", &"b".repeat(64), body).await.0,
        StatusCode::CONFLICT
    );
    let id = begun["actionId"].as_str().unwrap();
    let fleet = begun["fleetId"].as_str().unwrap();
    let poll = json!({"actionId":id});
    let waiting = f.post("association-status", &token, poll.clone()).await.1;
    assert_eq!(waiting["phase"], "awaiting_owner");
    assert!(waiting.get("profile").is_none());
    assert_eq!(
        f.post("association-status", &"b".repeat(64), poll.clone())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let owner = f.session("provider").await;
    let admin = f.session("admin").await;
    let other = f.session("manager").await;
    let view = f.call(&owner, "palpo.inbox.get", json!({"id":id})).await;
    assert_eq!(view.0, StatusCode::OK, "{view:?}");
    assert!(!view.1.to_string().contains(&token));
    assert!(!view.1.to_string().contains("secretDigest"));
    assert_eq!(
        f.call(&admin, "palpo.inbox.get", json!({"id":id})).await.0,
        StatusCode::NOT_FOUND
    );
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    let notices: Vec<_> = w.notices.values().filter(|n| n["actionId"] == id).collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0]["recipient"], "@provider:example.test");
    let confirm =
        json!({"id":id,"decision":"approve","commandId":"owner_confirm","expectedRevision":1});
    assert_ne!(
        f.call(&admin, "palpo.inbox.decide", confirm.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_ne!(
        f.call(&other, "palpo.inbox.decide", confirm.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(&owner, "palpo.inbox.decide", confirm.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(&owner, "palpo.inbox.decide", confirm).await.0,
        StatusCode::OK
    );
    let awaiting = f.post("association-status", &token, poll.clone()).await.1;
    assert_eq!(awaiting["phase"], "awaiting_admin");
    assert!(awaiting.get("profile").is_none());
    assert!(
        f.app.store.lock().await.read().unwrap()["fleets"]
            .get(fleet)
            .is_none()
    );
    let approve =
        json!({"id":id,"decision":"approve","commandId":"admin_confirm","expectedRevision":2});
    assert_ne!(
        f.call(&owner, "palpo.inbox.decide", approve.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(&admin, "palpo.inbox.decide", approve).await.0,
        StatusCode::OK
    );
    let approved = f.post("association-status", &token, poll.clone()).await.1;
    assert_eq!(approved["phase"], "awaiting_connection");
    assert_eq!(approved["profile"]["runtimeId"], "d".repeat(64));
    assert!(approved["profile"]["registration"]["as_token"].is_string());
    // A process restart reloads the same pairing authority from the private store.
    let state = f.app.store.lock().await.read().unwrap();
    let serialized = serde_json::to_value(Workflows::load(&state).unwrap()).unwrap();
    assert!(!serialized.to_string().contains(&token));
    let imported = f
        .post(
            "association-status",
            &token,
            json!({"actionId":id,"imported":true}),
        )
        .await
        .1;
    assert!(imported.get("profile").is_none());
    let connection = f
        .call(&owner, "palpo.fleets.connect", json!({"fleetId":fleet}))
        .await;
    assert_eq!(connection.0, StatusCode::OK, "{connection:?}");
    // Sending a probe alone is not successful end-to-end verification.
    assert_eq!(
        f.post("association-status", &token, poll.clone()).await.1["phase"],
        "awaiting_connection"
    );
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            state["fleets"][fleet]["transport"]["generation"] = json!(2);
            Ok(())
        })
        .unwrap();
    let rotated = f.post("association-status", &token, poll.clone()).await.1;
    assert_eq!(rotated["phase"], "unavailable");
    assert!(rotated.get("profile").is_none());
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            state["fleets"][fleet]["transport"]["generation"] = json!(1);
            let mut w = Workflows::load(state)?;
            w.authority.engagements.get_mut(fleet).unwrap().state = EngagementState::Suspended;
            w.save(state)
        })
        .unwrap();
    let suspended = f.post("association-status", &token, poll).await.1;
    assert_eq!(suspended["phase"], "unavailable");
    assert!(suspended.get("profile").is_none());
}

#[tokio::test]
async fn native_pairing_rejection_expiry_and_host_boundary_never_issue_credentials() {
    let f = Fixture::new().await;
    let token = "a".repeat(64);
    let owner = f.session("provider").await;
    let begun = f
        .post("association-start", &token, start_body("rejected_pair"))
        .await
        .1;
    let id = &begun["actionId"];
    let decision = f.call(&owner, "palpo.inbox.decide", json!({"id":id,"decision":"reject","reason":"Not my installation","commandId":"owner_reject","expectedRevision":1})).await;
    assert_eq!(decision.0, StatusCode::OK, "{decision:?}");
    let rejected = f
        .post("association-status", &token, json!({"actionId":id}))
        .await
        .1;
    assert_eq!(rejected["phase"], "rejected");
    assert!(rejected.get("profile").is_none());
    let begun = f
        .post("association-start", &token, start_body("expired_pair"))
        .await
        .1;
    let id = begun["actionId"].as_str().unwrap();
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            let w = Workflows::load(state)?;
            let mut raw = serde_json::to_value(w)?;
            raw["associations"][id]["pairing"]["ownerExpiresAtMs"] = json!(1);
            serde_json::from_value::<Workflows>(raw)?.save(state)
        })
        .unwrap();
    let expired = f
        .post("association-status", &token, json!({"actionId":id}))
        .await
        .1;
    assert_eq!(expired["phase"], "expired");
    assert!(expired.get("profile").is_none());
    assert_eq!(
        f.call(
            &owner,
            "palpo.inbox.decide",
            json!({"id":id,"decision":"approve","commandId":"late_owner","expectedRevision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let response = TestClient::post("http://operations.test/_palpo/miniapp/v1/association-start")
        .add_header("host", "operations.test", true)
        .add_header("origin", "https://hostile.test", true)
        .add_header("authorization", format!("Bearer {token}"), true)
        .json(&start_body("browser_request"))
        .send(&f.service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
}
