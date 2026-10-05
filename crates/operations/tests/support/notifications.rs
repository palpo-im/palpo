use palpo_operations::notifications::{Configuration, tick};

use super::*;

pub(super) fn config() -> Configuration {
    Configuration {
        bot: "@notices:example.test".to_owned().try_into().unwrap(),
        token: "notices-token".into(),
        public_origin: "https://example.test".into(),
        quiet_start: 0,
        quiet_end: 0,
    }
}

#[tokio::test]
async fn explicit_action_room_setup_joins_once_and_repairs_only_on_owner_request() {
    let f = Fixture::configured(true).await;
    let manager = f.session("manager").await;
    assert!(
        f.call(&manager, "palpo.actions.room.get", json!({}))
            .await
            .1["room"]
            .is_null()
    );
    assert_eq!(f.rooms.lock().unwrap().create_count, 0);
    let (code, room) = f
        .call(&manager, "palpo.actions.room.ensure", json!({}))
        .await;
    assert_eq!(code, StatusCode::OK, "{room}");
    assert_eq!(room["account"], "@manager:example.test");
    assert_eq!(room["revision"], 1);
    assert_eq!(
        f.call(&manager, "palpo.actions.room.ensure", json!({}))
            .await
            .1,
        room
    );
    assert_eq!(f.rooms.lock().unwrap().create_count, 1);
    let id = room["roomId"].as_str().unwrap();
    assert_eq!(
        f.call(&manager, "palpo.actions.room.get", json!({"roomId":id}))
            .await
            .1["room"],
        room
    );
    let provider = f.session("provider").await;
    assert!(
        f.call(&provider, "palpo.actions.room.get", json!({"roomId":id}))
            .await
            .1["room"]
            .is_null()
    );
    f.rooms.lock().unwrap().states.get_mut(id).unwrap().as_array_mut().unwrap().push(json!({"type":"m.room.member","state_key":"@intruder:example.test","content":{"membership":"join"}}));
    assert_eq!(
        f.call(&manager, "palpo.actions.room.get", json!({"roomId":id}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.rooms.lock().unwrap().create_count,
        1,
        "read cannot repair or re-invite"
    );
    let (code, repaired) = f
        .call(&manager, "palpo.actions.room.ensure", json!({}))
        .await;
    assert_eq!(code, StatusCode::OK, "{repaired}");
    assert_eq!(repaired["revision"], 2);
    assert_ne!(repaired["roomId"], room["roomId"]);
    assert_eq!(
        f.call(&manager, "palpo.actions.room.ensure", json!({}))
            .await
            .1,
        repaired
    );
    assert_eq!(f.rooms.lock().unwrap().create_count, 2);
    assert!(
        f.call(&manager, "palpo.actions.room.get", json!({"roomId":id}))
            .await
            .1["room"]
            .is_null()
    );
}
async fn request(f: &Fixture) -> String {
    let manager = f.session("manager").await;
    let (status, created) = f
        .call(&manager, "palpo.inbox.submit", project_request())
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    created["action"]["id"].as_str().unwrap().into()
}

#[tokio::test]
async fn account_preferences_replay_conflict_and_pause_delivery_without_resolving_actions() {
    let f = Fixture::new().await;
    let id = request(&f).await;
    let actor = f.session("coordinator").await;
    let manager = f.session("manager").await;
    let body = json!({"expectedRevision":0,"enabled":false,"remindersEnabled":false,
        "reminderMinutes":["15","30"],"quietHours":null});
    let (code, saved) = f
        .call(&actor, "palpo.notifications.set", body.clone())
        .await;
    assert_eq!(code, StatusCode::OK, "{saved}");
    assert_eq!(saved["revision"], 1);
    assert_eq!(
        f.call(&actor, "palpo.notifications.set", body.clone())
            .await
            .1,
        saved
    );
    assert_eq!(
        f.call(&manager, "palpo.notifications.get", json!({}))
            .await
            .1["revision"],
        0
    );
    let mut conflict = body.clone();
    conflict["enabled"] = json!(true);
    assert_eq!(
        f.call(&actor, "palpo.notifications.set", conflict.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    let now = now_ms();
    tick(&f.app, &config(), now).await.unwrap();
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    assert_eq!(w.actions[&id].state, "requested");
    assert!(
        w.notices
            .values()
            .filter(|n| n["recipient"] == "@coordinator:example.test")
            .all(|n| n["delivered"] == 0)
    );
    // Settings persist outside the borrowed app session. Reopening cannot reset
    // a disabled preference or grant a different actor access to it.
    let renewed = f.session("coordinator").await;
    assert_eq!(
        f.call(&renewed, "palpo.notifications.get", json!({}))
            .await
            .1,
        saved
    );
    conflict["expectedRevision"] = json!(1);
    assert_eq!(
        f.call(&renewed, "palpo.notifications.set", conflict.clone())
            .await
            .0,
        StatusCode::OK
    );
    tick(&f.app, &config(), now + 1000).await.unwrap();
    tick(&f.app, &config(), now + 901000).await.unwrap();
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    assert!(
        w.notices
            .values()
            .filter(|n| n["recipient"] == "@coordinator:example.test")
            .all(|n| n["delivered"] == 1)
    );
    conflict["expectedRevision"] = json!(2);
    conflict["remindersEnabled"] = json!(true);
    assert_eq!(
        f.call(&renewed, "palpo.notifications.set", conflict.clone())
            .await
            .0,
        StatusCode::OK
    );
    tick(&f.app, &config(), now + 902000).await.unwrap();
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    assert!(
        w.notices
            .values()
            .filter(|n| n["recipient"] == "@coordinator:example.test")
            .all(|n| n["delivered"] == 2)
    );
    conflict["expectedRevision"] = json!(3);
    conflict["quietHours"] = json!({"start":"22:00","end":"08:00","timeZone":"No/Such_Zone"});
    assert_eq!(
        f.call(&renewed, "palpo.notifications.set", conflict)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.call(&renewed, "palpo.notifications.get", json!({}))
            .await
            .1["revision"],
        3
    );
}

#[tokio::test]
async fn notices_retry_lost_reply_pin_board_and_seen_does_not_end_reminders() {
    let f = Fixture::new().await;
    let id = request(&f).await;
    let now = now_ms();
    f.rooms.lock().unwrap().lose_event_reply = true;
    tick(&f.app, &config(), now).await.unwrap();
    let before = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    assert!(
        before
            .notices
            .values()
            .any(|n| n["delivered"] == 0 && n["attempt"] == 1)
    );
    tick(&f.app, &config(), now + 3000).await.unwrap();
    let after = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    assert!(after.notices.values().all(|n| n["delivered"] == 1));
    {
        let rooms = f.rooms.lock().unwrap();
        assert_eq!(rooms.create_count, 2);
        assert_eq!(rooms.pins.len(), 2);
        let notices: Vec<_> = rooms
            .events
            .values()
            .filter(|e| e.get("im.palpo.action.v1").is_some())
            .collect();
        assert_eq!(
            notices.len(),
            2,
            "lost reply must reuse the Matrix transaction"
        );
        for notice in notices {
            assert_eq!(notice["im.palpo.action.v1"]["id"], id);
            assert!(notice.get("payload").is_none());
            assert!(!notice.to_string().contains("definitionDigest"));
        }
    }
    let coordinator = f.session("coordinator").await;
    assert_eq!(
        f.call(&coordinator, "palpo.inbox.seen", json!({"id":id}))
            .await
            .0,
        StatusCode::OK
    );
    tick(&f.app, &config(), now + 3600100).await.unwrap();
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    let notice = w
        .notices
        .values()
        .find(|n| n["recipient"] == "@coordinator:example.test")
        .unwrap();
    // The test delegation expires at one hour; stale coordinators receive no reminder.
    assert_eq!(notice["cancelled"], true);
    assert_eq!(w.actions[&id].state, "requested");
}

#[tokio::test]
async fn overdue_reminders_collapse_into_one_delivery_after_quiet_hours() {
    let f = Fixture::new().await;
    let id = request(&f).await;
    let now = now_ms();
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            let mut w = Workflows::load(state)?;
            w.authority
                .engagements
                .get_mut("engagement_a")
                .unwrap()
                .delegation_expires_at_ms = now + 7 * 86400000;
            w.save(state)
        })
        .unwrap();
    tick(&f.app, &config(), now + 3 * 86400000).await.unwrap();
    tick(&f.app, &config(), now + 3 * 86400000 + 60001)
        .await
        .unwrap();
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    let notice = w
        .notices
        .values()
        .find(|n| n["recipient"] == "@coordinator:example.test")
        .unwrap();
    assert_eq!(notice["delivered"], 1);
    assert_eq!(notice["finished"], true);
    assert_eq!(w.actions[&id].state, "requested");
}

#[tokio::test]
async fn private_notice_room_recovery_and_membership_refusal_leave_action_pending() {
    let f = Fixture::new().await;
    let id = request(&f).await;
    let now = now_ms();
    // Keep the coordinator current long enough to exercise reminder privacy.
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            let mut w = Workflows::load(state)?;
            w.authority
                .engagements
                .get_mut("engagement_a")
                .unwrap()
                .delegation_expires_at_ms = now + 86400000;
            w.save(state)
        })
        .unwrap();
    f.rooms.lock().unwrap().lose_create_reply = true;
    tick(&f.app, &config(), now).await.unwrap();
    tick(&f.app, &config(), now + 3000).await.unwrap();
    assert_eq!(f.rooms.lock().unwrap().create_count, 2);
    let coordinator = f.session("coordinator").await;
    f.call(&coordinator, "palpo.inbox.seen", json!({"id":id}))
        .await;
    tick(&f.app, &config(), now + 3600100).await.unwrap();
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    let n = w
        .notices
        .values()
        .find(|n| n["recipient"] == "@coordinator:example.test")
        .unwrap();
    assert_eq!(
        n["delivered"], 2,
        "seen is not an approval or a cancellation"
    );
    let room = n["roomId"].as_str().unwrap();
    f.rooms.lock().unwrap().states.get_mut(room).unwrap().as_array_mut().unwrap().push(json!({"type":"m.room.member","state_key":"@intruder:example.test","content":{"membership":"invite"}}));
    // Make the next reminder due while authority is current.
    f.app
        .store
        .lock()
        .await
        .transaction(|state| {
            let mut w = Workflows::load(state)?;
            for n in w
                .notices
                .values_mut()
                .filter(|n| n["recipient"] == "@coordinator:example.test")
            {
                n["dueAt"] = json!(now + 3600200);
            }
            w.save(state)
        })
        .unwrap();
    tick(&f.app, &config(), now + 3600200).await.unwrap();
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    let n = w
        .notices
        .values()
        .find(|n| n["recipient"] == "@coordinator:example.test")
        .unwrap();
    assert_eq!(n["delivered"], 2);
    assert_eq!(n["lastError"], "action_room_not_private");
    assert_eq!(w.actions[&id].state, "requested");
}

#[tokio::test]
async fn quiet_hours_delay_delivery_and_decision_cancels_old_revision_notices() {
    let f = Fixture::new().await;
    let id = request(&f).await;
    let now = now_ms();
    let mut quiet = config();
    quiet.quiet_start = ((now / 60000) % 1440) as u16;
    quiet.quiet_end = (quiet.quiet_start + 10) % 1440;
    tick(&f.app, &quiet, now).await.unwrap();
    assert_eq!(f.rooms.lock().unwrap().create_count, 0);
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    assert!(
        w.notices
            .values()
            .all(|n| n["dueAt"].as_u64().unwrap() > now && n["delivered"] == 0)
    );
    let coordinator = f.session("coordinator").await;
    let rejected=f.call(&coordinator,"palpo.inbox.decide",json!({"id":id,"decision":"reject","expectedRevision":1,"commandId":"reject_notice","reason":"Not this time"})).await;
    assert_eq!(rejected.0, StatusCode::OK, "{:?}", rejected);
    tick(&f.app, &config(), now_ms()).await.unwrap();
    let w = Workflows::load(&f.app.store.lock().await.read().unwrap()).unwrap();
    assert!(
        w.notices
            .values()
            .filter(|n| n["revision"] == 1)
            .all(|n| n["cancelled"] == true)
    );
    assert!(
        w.notices
            .values()
            .filter(|n| n["revision"] == 2)
            .all(|n| n["delivered"] == 1 && n["finished"] == true)
    );
    assert_eq!(w.actions[&id].state, "rejected");
    for event in f
        .rooms
        .lock()
        .unwrap()
        .events
        .values()
        .filter(|e| e.get("im.palpo.action.v1").is_some())
    {
        assert_eq!(event["im.palpo.action.v1"]["revision"], 2);
    }
}
