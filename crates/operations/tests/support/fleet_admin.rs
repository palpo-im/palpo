use super::*;

async fn registered(f: &Fixture, key: &str) -> String {
    let (_,requested)=f.post("association-request","provider-token",json!({"requestId":key,"name":key,"runtimeId":"e".repeat(64),"coordinatorMxid":"@coordinator:example.test","delegationExpiresAtMs":now_ms()+86400000,"exportMxids":["@coordinator:example.test"]})).await;
    let admin = f.session("admin").await;
    let result=f.call(&admin,"palpo.inbox.decide",json!({"id":requested["action"]["id"],"decision":"approve","commandId":format!("approve_{key}"),"expectedRevision":1})).await;
    assert_eq!(result.0, StatusCode::OK, "{result:?}");
    requested["action"]["fleetId"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn credential_controls_fence_before_io_recover_lost_reply_and_keep_history() {
    let f = Fixture::new().await;
    let fleet = registered(&f, "credential_controls").await;
    let admin = f.session("admin").await;
    let owner = f.session("provider").await;
    let pause = json!({"fleetId":fleet,"action":"pause","requestId":"pause_1"});
    assert_eq!(
        f.call(&owner, "palpo.fleets.set_state", pause.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let original = f.app.store.lock().await.read().unwrap()["fleets"][&fleet].clone();
    f.registrations.lock().unwrap().lose_control_reply = true;
    assert_eq!(
        f.call(&admin, "palpo.fleets.set_state", pause.clone())
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    let state = f.app.store.lock().await.read().unwrap();
    assert_eq!(state["fleets"][&fleet]["state"], "paused");
    assert_eq!(state["fleets"][&fleet]["pendingAdminOperation"], "pause_1");
    let workflows = palpo_operations::workflow::Workflows::load(&state).unwrap();
    let association = workflows
        .associations
        .values()
        .find(|a| a.fleet_id == fleet)
        .unwrap();
    assert_eq!(association.execution, "suspended");
    let paused_revision = association.revision;
    assert!(
        workflows
            .notices
            .values()
            .any(|n| n["actionId"] == association.id
                && n["revision"] == paused_revision
                && n["recipient"] == "@provider:example.test")
    );
    assert_eq!(
        state["fleets"][&fleet]["registration"],
        original["registration"]
    );
    assert!(
        palpo_operations::outbound::authenticate(
            &state,
            &fleet,
            original["transport"]["token"].as_str().unwrap(),
            Some(1),
            false
        )
        .is_err()
    );
    assert_eq!(
        f.call(
            &admin,
            "palpo.fleets.set_state",
            json!({"fleetId":fleet,"action":"resume","requestId":"different"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let done = f
        .call(&admin, "palpo.fleets.set_state", pause.clone())
        .await;
    assert_eq!(done.0, StatusCode::OK, "{done:?}");
    assert_eq!(done.1["fleet"]["canResume"], true);
    assert_eq!(done.1["fleet"]["connectionVerified"], false);
    assert_eq!(f.registrations.lock().unwrap().control_calls, 1);
    let resume = f
        .call(
            &admin,
            "palpo.fleets.set_state",
            json!({"fleetId":fleet,"action":"resume","requestId":"resume_1"}),
        )
        .await;
    assert_eq!(resume.0, StatusCode::OK, "{resume:?}");
    let state = f.app.store.lock().await.read().unwrap();
    let workflows = palpo_operations::workflow::Workflows::load(&state).unwrap();
    let association = workflows
        .associations
        .values()
        .find(|a| a.fleet_id == fleet)
        .unwrap();
    assert_eq!(association.execution, "verifying");
    assert!(association.revision > paused_revision);
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["fleets"][&fleet]["state"],
        "pending_connection"
    );
    assert_eq!(
        f.call(&admin, "palpo.fleets.set_state", pause).await.0,
        StatusCode::OK
    );
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["fleets"][&fleet]["state"],
        "pending_connection",
        "old successful pause replay must not pause again"
    );
    assert_eq!(f.registrations.lock().unwrap().control_calls, 2);
    let revoke = f
        .call(
            &admin,
            "palpo.fleets.set_state",
            json!({"fleetId":fleet,"action":"revoke","requestId":"revoke_1"}),
        )
        .await;
    assert_eq!(revoke.0, StatusCode::OK, "{revoke:?}");
    assert_eq!(
        revoke.1["fleet"]["revocationScope"],
        "appservice_credentials_only"
    );
    assert_eq!(revoke.1["fleet"]["localTaskStop"], "unconfirmed");
    assert_eq!(
        f.call(
            &admin,
            "palpo.fleets.set_state",
            json!({"fleetId":fleet,"action":"resume","requestId":"resume_2"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let state = f.app.store.lock().await.read().unwrap();
    assert_eq!(
        state["fleets"][&fleet]["registration"],
        original["registration"]
    );
    assert!(
        state["audit"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["action"] == "fleet.revoke")
    );
}

#[tokio::test]
async fn transport_rotation_reconciles_url_and_isolates_same_server_profiles_and_leases() {
    let f = Fixture::new().await;
    let first = registered(&f, "rotation_first").await;
    let second = registered(&f, "rotation_second").await;
    let admin = f.session("admin").await;
    let state = f.app.store.lock().await.read().unwrap();
    let original = state["fleets"][&first].clone();
    let other = state["fleets"][&second].clone();
    // Start from an installed legacy callback to exercise a lost atomic URL reply.
    f.registrations
        .lock()
        .unwrap()
        .values
        .get_mut(&first)
        .unwrap()["url"] = json!("https://previous.example.test/callback");
    f.app
        .store
        .lock()
        .await
        .transaction(|s| {
            s["fleets"][&first]["registration"]["url"] =
                json!("https://previous.example.test/callback");
            Ok(())
        })
        .unwrap();
    {
        let mut db = rusqlite::Connection::open(f._directory.path().join("admin.sqlite")).unwrap();
        let tx = db.transaction().unwrap();
        for id in ["acked", "unacked"] {
            palpo_operations::outbound::enqueue(
                &tx,
                &original,
                "work",
                "request",
                id,
                &json!({"request":id}),
                Default::default(),
            )
            .unwrap();
        }
        tx.execute(
            "UPDATE fleet_delivery SET acked=?1 WHERE fleet=?2 AND id='acked'",
            rusqlite::params![now_ms(), first],
        )
        .unwrap();
        let delivered = palpo_operations::outbound::claim(
            &tx,
            &original,
            "work",
            "11111111-1111-4111-8111-111111111111",
            now_ms(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(delivered["delivery"]["id"], "unacked");
        tx.commit().unwrap();
    }
    let args = json!({"fleetId":first,"requestId":"rotate_1","rotate":true});
    f.registrations.lock().unwrap().lose_url_reply = true;
    assert_eq!(
        f.call(&admin, "palpo.fleets.migrate", args.clone()).await.0,
        StatusCode::BAD_GATEWAY
    );
    let fenced = f.app.store.lock().await.read().unwrap();
    assert_eq!(fenced["fleets"][&first]["state"], "rotating");
    assert_eq!(fenced["fleets"][&first]["transport"], original["transport"]);
    assert_eq!(
        f.call(
            &admin,
            "palpo.fleets.migrate",
            json!({"fleetId":first,"requestId":"rotate_1","rotate":false})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let result = f.call(&admin, "palpo.fleets.migrate", args.clone()).await;
    assert_eq!(result.0, StatusCode::OK, "{result:?}");
    let listed = f.call(&admin, "palpo.fleets.list", json!({})).await.1;
    let projected = listed["fleets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == first)
        .unwrap();
    assert_eq!(projected["transportGeneration"], 2);
    assert_eq!(projected["registrationGeneration"], 1);
    let state = f.app.store.lock().await.read().unwrap();
    let updated = &state["fleets"][&first];
    assert_eq!(updated["transport"]["generation"], 2);
    assert_eq!(updated["registrationGeneration"], 1);
    assert_ne!(
        updated["transport"]["token"],
        original["transport"]["token"]
    );
    assert_eq!(state["fleets"][&second], other);
    assert_eq!(f.registrations.lock().unwrap().url_calls, 1);
    assert!(
        palpo_operations::outbound::authenticate(
            &state,
            &first,
            original["transport"]["token"].as_str().unwrap(),
            Some(1),
            false
        )
        .is_err()
    );
    assert!(
        palpo_operations::outbound::authenticate(
            &state,
            &second,
            other["transport"]["token"].as_str().unwrap(),
            Some(1),
            false
        )
        .is_ok()
    );
    {
        let db = rusqlite::Connection::open(f._directory.path().join("admin.sqlite")).unwrap();
        let row:(u64,Option<String>,Option<String>)=db.query_row("SELECT generation,consumer,token FROM fleet_delivery WHERE fleet=?1 AND id='unacked'",[&first],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(row, (2, None, None));
        let generation: u64 = db
            .query_row(
                "SELECT generation FROM fleet_delivery WHERE fleet=?1 AND id='acked'",
                [&first],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(generation, 1);
    }
    assert_eq!(
        f.call(&admin, "palpo.fleets.migrate", args).await.0,
        StatusCode::OK
    );
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["fleets"][&first]["transport"],
        updated["transport"]
    );
    let text = result.1.to_string();
    for secret in [
        original["transport"]["token"].as_str().unwrap(),
        updated["transport"]["token"].as_str().unwrap(),
        updated["registration"]["as_token"].as_str().unwrap(),
    ] {
        assert!(!text.contains(secret));
    }
}

#[tokio::test]
async fn administrator_activity_is_bounded_and_omits_secret_extensions() {
    let f = Fixture::new().await;
    f.app.store.lock().await.transaction(|s|{s["audit"]=json!((0..210).map(|i|json!({"atMs":now_ms(),"actor":"@admin:example.test","action":"fixture","result":i.to_string(),"token":"private-extension"})).collect::<Vec<_>>());Ok(())}).unwrap();
    let admin = f.session("admin").await;
    let member = f.session("manager").await;
    assert_eq!(
        f.call(&member, "palpo.activity.list", json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    let result = f.call(&admin, "palpo.activity.list", json!({})).await;
    assert_eq!(result.0, StatusCode::OK);
    assert_eq!(result.1["events"].as_array().unwrap().len(), 200);
    assert_eq!(result.1["events"][0]["result"], "209");
    assert!(!result.1.to_string().contains("private-extension"));
}
