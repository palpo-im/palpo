//! Matrix HTTP fixtures for retry/authority tests; these do not claim live
//! homeserver or Hagency execution coverage.
use std::collections::BTreeMap;
use std::sync::Mutex;

use palpo_operations::workflow::Request;

use super::*;

#[derive(Default)]
pub(super) struct Rooms {
    pub(super) states: BTreeMap<String, Value>,
    aliases: BTreeMap<String, String>,
    pub(super) events: BTreeMap<String, Value>,
    pub(super) lose_create_reply: bool,
    lose_binding_reply: bool,
    pub(super) lose_event_reply: bool,
    pub(super) create_count: usize,
    pub(super) pins: BTreeMap<String, Value>,
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
    let path = req.uri().path().to_owned();
    if !(path.ends_with("/createRoom")
        || path.ends_with("/joined_rooms")
        || path.contains("/directory/room/")
        || path.contains("/rooms/")
        || path.contains("/join/"))
    {
        return false;
    }
    let token = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let appservice = depot
        .get_typed::<Arc<Mutex<association_cases::Registrations>>>()
        .unwrap()
        .lock()
        .unwrap()
        .values
        .values()
        .any(|v| format!("Bearer {}", v["as_token"].as_str().unwrap()) == token);
    if token != "Bearer manager-token"
        && token != "Bearer manager-new-token"
        && token != "Bearer fixture-as"
        && token != "Bearer notices-token"
        && token != "Bearer provider-token"
        && !appservice
    {
        return false;
    }
    let body = if req.method() != reqwest::Method::GET {
        req.parse_json::<Value>().await.unwrap_or(json!({}))
    } else {
        Value::Null
    };
    let mut rooms = depot
        .get_typed::<Arc<Mutex<Rooms>>>()
        .unwrap()
        .lock()
        .unwrap();
    if path.ends_with("/createRoom") {
        let creator = if token == "Bearer notices-token" {
            "@notices:example.test"
        } else if token == "Bearer provider-token" {
            "@provider:example.test"
        } else {
            "@manager:example.test"
        };
        rooms.create_count += 1;
        let id = format!("!room{}:example.test", rooms.create_count);
        let mut state = body["initial_state"].as_array().unwrap().clone();
        state.extend([
            json!({"type":"m.room.create","state_key":"","sender":creator,"content":body.get("creation_content").cloned().unwrap_or(json!({"room_version":"11"}))}),
            json!({"type":"m.room.member","state_key":creator,"content":{"membership":"join"}}),
            json!({"type":"m.room.power_levels","state_key":"","content":body["power_level_content_override"]}),
        ]);
        for invite in body["invite"].as_array().unwrap() {
            state.push(json!({"type":"m.room.member","state_key":invite,"content":{"membership":"invite"}}));
        }
        rooms.states.insert(id.clone(), json!(state));
        if std::mem::take(&mut rooms.lose_create_reply) {
            // Server committed the room, but alias creation/response was lost.
            res.status_code(StatusCode::BAD_GATEWAY);
            res.render(Json(json!({"errcode":"M_UNKNOWN"})));
            return true;
        }
        let alias = format!(
            "#{}:example.test",
            body["room_alias_name"].as_str().unwrap()
        );
        rooms.aliases.insert(
            self::path(&["_matrix", "client", "v3", "directory", "room", &alias]),
            id.clone(),
        );
        res.render(Json(json!({"room_id":id})));
        return true;
    }
    if path.ends_with("/joined_rooms") {
        res.render(Json(
            json!({"joined_rooms":rooms.states.keys().collect::<Vec<_>>()}),
        ));
        return true;
    }
    if path.contains("/directory/room/") {
        if let Some(id) = rooms.aliases.get(&path) {
            res.render(Json(json!({"room_id":id})));
        } else {
            res.status_code(StatusCode::NOT_FOUND);
            res.render(Json(json!({"errcode":"M_NOT_FOUND"})));
        }
        return true;
    }
    if path.contains("/join/")
        && (appservice || token == "Bearer manager-token" || token == "Bearer manager-new-token")
    {
        let user = if appservice {
            req.query::<String>("user_id").unwrap()
        } else {
            "@manager:example.test".into()
        };
        let (room, state) = rooms
            .states
            .iter_mut()
            .find(|(id, _)| self::path(&["_matrix", "client", "v3", "join", id]) == path)
            .unwrap();
        let member = state
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|e| e["type"] == "m.room.member" && e["state_key"] == user)
            .unwrap();
        member["content"]["membership"] = json!("join");
        res.render(Json(json!({"room_id":room})));
        return true;
    }
    if path.contains("/send/com.hagency.connection.probe.v1/") && appservice {
        if let Some(existing) = rooms.events.get(&path) {
            assert_eq!(existing, &body);
        } else {
            rooms.events.insert(path.clone(), body);
        }
        if std::mem::take(&mut rooms.lose_event_reply) {
            res.status_code(StatusCode::BAD_GATEWAY);
            res.render(Json(json!({"errcode":"M_UNKNOWN"})));
            return true;
        }
        res.render(Json(json!({"event_id":"$connection-probe"})));
        return true;
    }
    if req.method() == reqwest::Method::PUT && path.contains("/state/") {
        for (room, state) in rooms.states.iter_mut() {
            for (kind, key) in [
                ("com.hagency.admin.binding.v1", "engagement_a"),
                ("m.room.power_levels", ""),
            ] {
                if path
                    == self::path(&["_matrix", "client", "v3", "rooms", room, "state", kind, key])
                {
                    let entries = state.as_array_mut().unwrap();
                    if let Some(event) = entries
                        .iter_mut()
                        .find(|e| e["type"] == kind && e["state_key"] == key)
                    {
                        event["content"] = body.clone();
                    } else {
                        entries.push(json!({"type":kind,"state_key":key,"content":body}));
                    }
                    if kind == "com.hagency.admin.binding.v1"
                        && std::mem::take(&mut rooms.lose_binding_reply)
                    {
                        res.status_code(StatusCode::BAD_GATEWAY);
                        res.render(Json(json!({"errcode":"M_UNKNOWN"})));
                    } else {
                        res.render(Json(json!({})));
                    }
                    return true;
                }
            }
        }
    }
    if path.ends_with("/invite")
        && let Some((_, state)) = rooms
            .states
            .iter_mut()
            .find(|(id, _)| self::path(&["_matrix", "client", "v3", "rooms", id, "invite"]) == path)
    {
        state.as_array_mut().unwrap().push(json!({"type":"m.room.member","state_key":body["user_id"],"content":{"membership":"invite"}}));
        res.render(Json(json!({})));
        return true;
    }
    if let Some(state) = rooms
        .states
        .iter()
        .find(|(id, _)| self::path(&["_matrix", "client", "v3", "rooms", id, "state"]) == path)
        .map(|(_, v)| v)
    {
        res.render(Json(state.clone()));
        return true;
    }
    if path.contains("/send/com.hagency.engagement.request.v1/") {
        assert!(matches!(
            token.as_str(),
            "Bearer manager-token" | "Bearer manager-new-token"
        ));
        assert!(body.get("ownerDmRoomId").is_none());
        assert!(body.get("sourceEventId").is_none());
        if let Some(existing) = rooms.events.get(&path) {
            assert_eq!(
                existing, &body,
                "Matrix transaction reused with changed content"
            );
        } else {
            rooms.events.insert(path.clone(), body);
        }
        if std::mem::take(&mut rooms.lose_event_reply) {
            res.status_code(StatusCode::BAD_GATEWAY);
            res.render(Json(json!({"errcode":"M_UNKNOWN"})));
            return true;
        }
        res.render(Json(json!({"event_id":"$created-request"})));
        return true;
    }
    if path.ends_with("/messages") {
        let events:Vec<Value>=rooms.events.iter().filter(|(p,_)|p.contains("/send/com.hagency.engagement.request.v1/"))
            .map(|(_,content)|json!({"event_id":"$created-request","sender":"@manager:example.test","type":"com.hagency.engagement.request.v1","content":content})).collect();
        res.render(Json(json!({"chunk":events})));
        return true;
    }
    if path.contains("/send/m.room.message/") {
        assert_eq!(token, "Bearer notices-token");
        if let Some(existing) = rooms.events.get(&path) {
            assert_eq!(existing, &body, "changed notification transaction");
        } else {
            rooms.events.insert(path.clone(), body);
        }
        if std::mem::take(&mut rooms.lose_event_reply) {
            res.status_code(StatusCode::BAD_GATEWAY);
            res.render(Json(json!({"errcode":"M_UNKNOWN"})));
            return true;
        }
        res.render(Json(json!({"event_id":format!("$notice-{}",&palpo_operations::digest(&json!(path)).unwrap()[..24])})));
        return true;
    }
    if path.contains("/state/m.room.pinned_events/") {
        rooms.pins.insert(path, body);
        res.render(Json(json!({})));
        return true;
    }
    // Leave legacy room membership fixture handling to the original stub.
    false
}

const RESOURCE: &str = "resource_0123456789abcdef01234567";
async fn funded(f: &Fixture) {
    f.app.store.lock().await.transaction(|state|{
        let mut w=Workflows::load(state)?;
        w.resource_details.insert("grant_a".into(),json!({"resourceId":RESOURCE,"period":"monthly","periodKey":"2026-10"}));
        state["fleets"]["engagement_a"]=json!({"id":"engagement_a","name":"Provider","installation":"installed","state":"ready","registrationGeneration":1,
            "registration":{"as_token":"fixture-as","hs_token":"fixture-relay"},"representativeMxid":"@engagement_a_representative:example.test",
            "receptionRoomId":"!reception:example.test","transport":{"mode":"outbound","generation":1,"token":"fixture-machine"},
            "capabilities":{"coordinatorApprovalV1":true,"offers":[{"role":"developer","resources":[{"id":RESOURCE,"name":"Code resource","model":"model-a","framework":"fixture"}]}]}});
        w.save(state)
    }).unwrap();
    f.rooms.lock().unwrap().states.insert("!reception:example.test".into(),json!([
        {"type":"m.room.member","state_key":"@manager:example.test","content":{"membership":"join"}}
    ]));
}
fn project() -> Value {
    json!({"kind":"project","fleetId":"engagement_a","requestId":"form_project","resourceIds":["grant_a"],"name":"Project one","reason":"Build a useful app"})
}

#[tokio::test]
async fn catalog_requires_funded_eligible_engagement_allocation() {
    let f = Fixture::new().await;
    funded(&f).await;
    let manager = f.session("manager").await;
    let (status, rows) = f.call(&manager, "palpo.catalog.list", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    let resource = &rows["fleets"][0]["capabilities"]["offers"][0]["resources"][0];
    assert_eq!(resource["id"], RESOURCE);
    assert_eq!(resource["allocationId"], "grant_a");
    let admin = f.session("admin").await;
    assert_eq!(
        f.call(&admin, "palpo.catalog.list", json!({})).await.1["fleets"],
        json!([])
    );
    let denied = f.call(&admin, "palpo.inbox.submit", project()).await;
    assert_eq!(denied.0, StatusCode::FORBIDDEN, "{:?}", denied);
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            let mut w = Workflows::load(state)?;
            w.authority
                .resources
                .get_mut("grant_a")
                .unwrap()
                .allocated_tokens = 0.try_into()?;
            w.save(state)
        })
        .unwrap();
    assert_eq!(
        f.call(&manager, "palpo.catalog.list", json!({})).await.1["fleets"],
        json!([])
    );
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", project()).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(f.rooms.lock().unwrap().create_count, 0);
}

#[tokio::test]
async fn project_reconciles_committed_room_after_lost_response_and_freezes_definition() {
    let f = Fixture::new().await;
    funded(&f).await;
    let manager = f.session("manager").await;
    f.rooms.lock().unwrap().lose_create_reply = true;
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", project()).await.0,
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(f.rooms.lock().unwrap().create_count, 1);
    let (status, created) = f.call(&manager, "palpo.inbox.submit", project()).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["action"]["state"], "requested");
    assert_eq!(created["action"]["payload"]["reason"], "Build a useful app");
    assert_eq!(f.rooms.lock().unwrap().create_count, 2);
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", project()).await.1,
        created
    );
    let mut changed = project();
    changed["name"] = json!("Different project");
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", changed).await.0,
        StatusCode::CONFLICT
    );
    let id = created["action"]["id"].as_str().unwrap();
    assert_eq!(f.call(&manager,"palpo.inbox.decide",json!({"id":id,"decision":"approve","expectedRevision":1,"commandId":"self","reason":""})).await.0,StatusCode::FORBIDDEN);
    let coordinator = f.session("coordinator").await;
    let result=f.call(&coordinator,"palpo.inbox.decide",json!({"id":id,"decision":"approve","expectedRevision":1,"commandId":"project_approval","reason":""})).await;
    assert_eq!(result.0, StatusCode::OK, "{:?}", result);
    let state = f.app.store.lock().await.read().unwrap();
    let w = Workflows::load(&state).unwrap();
    assert_eq!(w.outbox["project_approval"]["queued"], true);
    assert_eq!(
        w.outbox["project_approval"]["command"]["request"]["resourceAllocations"],
        json!(["grant_a"])
    );
    assert_eq!(f.rooms.lock().unwrap().create_count, 2);
}

#[tokio::test]
async fn agent_form_uses_current_project_and_exact_idempotent_matrix_event() {
    let f = Fixture::new().await;
    funded(&f).await;
    let manager = f.session("manager").await;
    let (status, created) = f.call(&manager, "palpo.inbox.submit", project()).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let action = created["action"]["id"].as_str().unwrap();
    // Simulate the authenticated runtime's project grant; provisioning is covered
    // by the native integration suite, not by this Matrix transport fixture.
    let project_id = f
        .app
        .store
        .lock()
        .await
        .transaction(|state| {
            let mut w = Workflows::load(state)?;
            let Request::Project(r) = &w.actions[action].request else {
                panic!()
            };
            let id = r.project_id.as_str().to_owned();
            w.authority.projects.insert(
                id.clone(),
                ProjectGrant {
                    project_id: r.project_id.clone(),
                    server_engagement_id: r.server_engagement_id.clone(),
                    revision: r.revision,
                    owner: r.owner.clone(),
                    resource_allocations: r.resource_allocations.clone(),
                    state: ProjectState::Ready,
                },
            );
            w.save(state)?;
            Ok(id)
        })
        .unwrap();
    let intent = json!({"requestId":"form_agent","projectId":project_id,"role":"developer","resourceAllocationId":"grant_a","requestedTokens":"100000","ratePerDay":"10000",
        "agentDefinition":{"name":"Coder","resourceId":RESOURCE}});
    f.rooms.lock().unwrap().lose_event_reply = true;
    assert_eq!(
        f.call(&manager, "palpo.requests.create", intent.clone())
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    let (status, opened) = f
        .post(
            "session",
            "manager-new-token",
            json!({"appId":api::APP_ID,"bundleDigest":"c".repeat(64),"services":api::SERVICES}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let manager = opened["sessionToken"].as_str().unwrap().to_owned();
    let (status, agent) = f
        .call(&manager, "palpo.requests.create", intent.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{agent}");
    assert_eq!(
        f.call(&manager, "palpo.requests.create", intent.clone())
            .await
            .1,
        agent
    );
    assert_eq!(f.rooms.lock().unwrap().events.len(), 1);
    let state = f.app.store.lock().await.read().unwrap();
    let w = Workflows::load(&state).unwrap();
    let Request::Agent(r) = &w.actions[agent["action"]["id"].as_str().unwrap()].request else {
        panic!()
    };
    assert_eq!(r.resource_allocation_id.as_str(), "grant_a");
    let definition = &w.definitions[&String::from(r.definition_digest.clone())];
    assert_eq!(definition["sourceEventId"], "$created-request");
    assert_eq!(definition["requestedTokens"], 100000);
    assert_eq!(definition["ratePerDay"], 10000);
    assert_eq!(definition["ownerMxid"], "@manager:example.test");
    assert!(
        definition["ownerDmRoomId"]
            .as_str()
            .unwrap()
            .starts_with('!')
    );
    let mut forged = intent.clone();
    forged["ownerMxid"] = json!("@admin:example.test");
    assert_eq!(
        f.call(&manager, "palpo.requests.create", forged).await.0,
        StatusCode::BAD_REQUEST
    );
    let mut duplicate = intent;
    duplicate["requestId"] = json!("another_agent");
    assert_eq!(
        f.call(&manager, "palpo.requests.create", duplicate).await.0,
        StatusCode::CONFLICT
    );
    let coordinator = f.session("coordinator").await;
    let approved=f.call(&coordinator,"palpo.inbox.decide",json!({"id":agent["action"]["id"],"decision":"approve","expectedRevision":1,"commandId":"agent_approval","reason":""})).await;
    assert_eq!(approved.0, StatusCode::OK, "{:?}", approved);
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    assert_eq!(w.outbox["agent_approval"]["queued"], true);
}

#[tokio::test]
async fn selected_project_room_is_owner_checked_and_recovered_after_lost_binding_reply() {
    let f = Fixture::new().await;
    funded(&f).await;
    let manager = f.session("manager").await;
    let room = "!selected:example.test";
    let state = json!([
        {"type":"m.room.create","state_key":"","sender":"@manager:example.test","content":{"room_version":"11"}},
        {"type":"m.room.member","state_key":"@manager:example.test","content":{"membership":"join"}},
        {"type":"m.room.join_rules","state_key":"","content":{"join_rule":"invite"}},
        {"type":"m.room.power_levels","state_key":"","content":{"users":{"@manager:example.test":100},"invite":50}}
    ]);
    let mut intent = project();
    intent["roomId"] = json!(room);
    for bad in [
        json!({"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}),
        json!({"type":"com.hagency.admin.binding.v1","state_key":"other_fleet","content":{"projectId":"other"}}),
        json!({"type":"m.room.tombstone","state_key":"","content":{"replacement_room":"!new:example.test"}}),
    ] {
        let mut invalid = state.clone();
        invalid.as_array_mut().unwrap().push(bad);
        f.rooms
            .lock()
            .unwrap()
            .states
            .insert(room.into(), invalid.clone());
        assert_eq!(
            f.call(&manager, "palpo.inbox.submit", intent.clone())
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(f.rooms.lock().unwrap().states[room], invalid);
    }
    for field in ["creator", "power", "membership", "join_rule"] {
        let mut invalid = state.clone();
        match field {
            "creator" => invalid[0]["sender"] = json!("@other:example.test"),
            "power" => invalid[3]["content"]["users"]["@manager:example.test"] = json!(50),
            "membership" => invalid[1]["content"]["membership"] = json!("leave"),
            _ => invalid[2]["content"]["join_rule"] = json!("public"),
        }
        f.rooms
            .lock()
            .unwrap()
            .states
            .insert(room.into(), invalid.clone());
        assert_eq!(
            f.call(&manager, "palpo.inbox.submit", intent.clone())
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(f.rooms.lock().unwrap().states[room], invalid);
    }
    {
        let mut rooms = f.rooms.lock().unwrap();
        rooms.states.insert(room.into(), state);
        rooms.lose_binding_reply = true;
    }
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", intent.clone())
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(f.rooms.lock().unwrap().create_count, 0);
    let (status, created) = f.call(&manager, "palpo.inbox.submit", intent.clone()).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["action"]["payload"]["roomId"], room);
    assert_eq!(
        f.rooms.lock().unwrap().create_count,
        1,
        "only private approval room is created"
    );
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", intent.clone())
            .await
            .1,
        created
    );
    intent["roomId"] = json!("!different:example.test");
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", intent).await.0,
        StatusCode::CONFLICT
    );
    let mut other = project();
    other["requestId"] = json!("other_project");
    other["roomId"] = json!(room);
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", other).await.0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn legacy_inbox_continuation_preserves_identity_history_and_current_authority() {
    let f = Fixture::new().await;
    funded(&f).await;
    let actor = "@manager:example.test";
    let request = "legacy_pending_project";
    let id = format!(
        "action_{}",
        &palpo_operations::digest(&json!({"actor":actor,"requestId":request})).unwrap()[..32]
    );
    let old = json!({"id":id,"requestId":request,"kind":"project","ownerMxid":actor,"state":"approved","execution":"pending","revision":2,
        "createdAt":10,"updatedAt":11,"decision":{"by":"@admin:example.test","at":11,"reason":"Original admin decision"},
        "payload":{"name":"Original project","reason":"Original purpose","fleetId":"engagement_a","resourceIds":[RESOURCE]},"command":{"private":"never expose"}});
    f.app.store.lock().await.transaction(|s| {
        s["actionInbox"]=json!({"records":{id.clone():old.clone(),"old_contribution":{"id":"old_contribution","requestId":"old_contribution","kind":"contribution","ownerMxid":actor,
            "state":"rejected","execution":"pending","revision":2,"payload":{"name":"Earlier capacity"},"decision":{"by":"@admin:example.test","reason":"Original refusal"}}}});
        Ok(())
    }).unwrap();
    let manager = f.session("manager").await;
    let coordinator = f.session("coordinator").await;
    let admin = f.session("admin").await;
    let (status, opened) = f.call(&manager, "palpo.inbox.get", json!({"id":id})).await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    assert_eq!(opened["action"]["nextAction"], "continue_legacy_project");
    assert_eq!(opened["action"]["decision"], old["decision"]);
    assert!(opened["action"].get("command").is_none());
    assert_eq!(
        f.call(&admin, "palpo.inbox.get", json!({"id":id})).await.0,
        StatusCode::NOT_FOUND
    );
    let history = f
        .call(&manager, "palpo.inbox.list", json!({"view":"history"}))
        .await
        .1;
    assert!(
        history["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "old_contribution")
    );
    let intent = opened["action"]["continuation"].clone();
    assert_eq!(intent["requestId"], request);
    assert_eq!(intent["resourceIds"], json!(["grant_a"]));
    let mut forged = intent.clone();
    forged["name"] = json!("Changed name");
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", forged).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.call(&coordinator, "palpo.inbox.submit", intent.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(f.rooms.lock().unwrap().create_count, 0);
    f.rooms.lock().unwrap().lose_create_reply = true;
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", intent.clone())
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    let (status, continued) = f.call(&manager, "palpo.inbox.submit", intent.clone()).await;
    assert_eq!(status, StatusCode::OK, "{continued}");
    assert_eq!(continued["action"]["id"], id);
    assert_eq!(continued["action"]["state"], "requested");
    assert_eq!(
        f.call(&manager, "palpo.inbox.submit", intent.clone())
            .await
            .1,
        continued
    );
    let decision = json!({"id":id,"decision":"approve","expectedRevision":1,"commandId":"continue_legacy_approval","reason":"Current coordinator decision"});
    assert_eq!(
        f.call(&admin, "palpo.inbox.decide", decision.clone())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.call(&coordinator, "palpo.inbox.decide", decision.clone())
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(&coordinator, "palpo.inbox.decide", decision).await.0,
        StatusCode::OK
    );
    let state = f.app.store.lock().await.read().unwrap();
    let w = Workflows::load(&state).unwrap();
    assert_eq!(state["actionInbox"]["records"][&id], old);
    assert_eq!(w.legacy_sources[&id]["originalAction"], old);
    assert_eq!(w.outbox.len(), 1);
    assert_eq!(f.rooms.lock().unwrap().create_count, 2);
    assert_eq!(
        w.view(&id, &actor.to_owned().try_into().unwrap(), now_ms())
            .unwrap()["state"],
        "approved"
    );
}

#[tokio::test]
async fn legacy_agent_history_is_scoped_and_old_links_follow_adoption() {
    let f = Fixture::new().await;
    funded(&f).await;
    let old = json!({"id":"engagement_a:request_agent","requestId":"request_agent","fleetId":"engagement_a",
        "projectId":"existing_project","state":"retired","decision":{"by":"@provider:example.test","reason":"Old decision"},
        "payload":{"agentDefinition":{"name":"Earlier agent","secret":"PRIVATE"},"role":"coding","requestedTokens":100000},
        "provider":{"credential":"PRIVATE"}});
    f.app.store.lock().await.transaction(|state| {
        state["projects"]["existing_project"] = json!({"id":"existing_project","fleetId":"engagement_a","ownerMxid":"@manager:example.test"});
        state["requests"] = json!({"engagement_a:request_agent":old.clone()});
        Ok(())
    }).unwrap();
    let manager = f.session("manager").await;
    let admin = f.session("admin").await;
    let coordinator = f.session("coordinator").await;
    for session in [&manager, &coordinator] {
        let (status, list) = f.call(session, "palpo.requests.list", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            list["requests"][0]["agentDefinition"]["name"],
            "Earlier agent"
        );
        assert_eq!(list["requests"][0]["canRetire"], false);
        assert_eq!(list["requests"][0]["usable"], false);
        assert!(!list.to_string().contains("PRIVATE"));
    }
    assert_eq!(
        f.call(&admin, "palpo.requests.list", json!({})).await.1["total"],
        0
    );
    let opened = f
        .call(
            &manager,
            "palpo.inbox.get",
            json!({"id":"engagement_a:request_agent"}),
        )
        .await
        .1;
    assert_eq!(
        opened["action"]["decision"],
        json!({"by":"@provider:example.test","at":null,"reason":"Old decision"})
    );
    let history = f
        .call(&manager, "palpo.inbox.list", json!({"view":"history"}))
        .await
        .1;
    assert!(
        history["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "engagement_a:request_agent")
    );
    // Simulate the already-audited adoption's current action, never a second
    // runtime or new identifier. The old link resolves the current authority.
    let created = f
        .call(&manager, "palpo.inbox.submit", agent_request())
        .await
        .1;
    let current = &created["action"]["id"];
    assert!(current.is_string(), "{created}");
    assert_eq!(
        f.call(
            &manager,
            "palpo.inbox.get",
            json!({"id":"engagement_a:request_agent"})
        )
        .await
        .1["action"]["id"],
        *current
    );
    assert_eq!(
        f.call(&manager, "palpo.requests.list", json!({})).await.1["total"],
        1
    );
    assert_eq!(
        f.app.store.lock().await.read().unwrap()["requests"]["engagement_a:request_agent"],
        old
    );
}

#[tokio::test]
async fn catalog_capacity_is_scoped_current_and_never_inferred_from_total() {
    let f = Fixture::new().await;
    funded(&f).await;
    let manager = f.session("manager").await;
    let read =
        |value: Value| value["fleets"][0]["capabilities"]["offers"][0]["resources"][0].clone();
    let initial = read(f.call(&manager, "palpo.catalog.list", json!({})).await.1);
    assert!(initial["remainingTokens"].is_null());
    assert_eq!(initial["capacityState"], "unavailable");
    let now = now_ms();
    for (change, state) in [
        None,
        Some(("revision", json!(2))),
        Some(("resourceId", json!("another_resource"))),
        Some(("periodKey", json!("2026-09"))),
        Some(("stale", json!(true))),
    ]
    .into_iter()
    .zip([
        "current",
        "unavailable",
        "unavailable",
        "unavailable",
        "stale",
    ]) {
        f.app.store.lock().await.transaction(|stored| {
            let cap = &mut stored["fleets"]["engagement_a"]["capabilities"];
            cap["resourceBudgets"] = json!({"grant_a":{"resourceId":RESOURCE,"revision":1,"allocatedTokens":1000000,
                "retainedTokens":300000,"remainingTokens":700000,"overdrawn":false,"period":"monthly","periodKey":"2026-10"}});
            cap["resourceBudgetObservedAtMs"] = json!(now);
            cap["resourceBudgetReceivedAtMs"] = json!(now);
            if let Some((key,value)) = &change {
                if *key == "stale" { cap["resourceBudgetObservedAtMs"] = json!(now-90001); }
                else { cap["resourceBudgets"]["grant_a"][key] = value.clone(); }
            }
            Ok(())
        }).unwrap();
        let row = read(f.call(&manager, "palpo.catalog.list", json!({})).await.1);
        assert_eq!(row["capacityState"], state);
        assert_eq!(row["allocatedTokens"], 1000000);
        if state == "current" {
            assert_eq!(row["remainingTokens"], 700000);
        } else {
            assert!(row["remainingTokens"].is_null());
            assert!(row["retainedTokens"].is_null());
        }
    }
}
