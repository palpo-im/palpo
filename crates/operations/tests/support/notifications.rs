use palpo_operations::notifications::{Configuration, tick};

use super::*;

fn config() -> Configuration {
    Configuration {
        bot: "@notices:example.test".to_owned().try_into().unwrap(),
        token: "notices-token".into(),
        public_origin: "https://example.test".into(),
        quiet_start: 0,
        quiet_end: 0,
    }
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
