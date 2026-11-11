//! Isolated real PostgreSQL coverage and fixed-data selector measurements.
use std::sync::{Arc, Mutex};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Json, Text};
use horme::RunQueryDsl;
use serde_json::{Value, json};

use super::*;
use crate::test_database::sql_spy::SqlSpy;

const LOCAL: &str = "selectors.example:8448";
const REMOTE: &str = "recipient.example";

pub(super) static INITIAL_SWEEP_FINISHED: tokio::sync::Notify = tokio::sync::Notify::const_new();

struct EmptyProbe {
    kind: OutgoingKind,
    reached: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
}

static EMPTY_PROBE: Mutex<Option<EmptyProbe>> = Mutex::new(None);

pub(super) async fn pause_empty_probe(kind: &OutgoingKind) {
    let probe = {
        let mut probe = EMPTY_PROBE.lock().unwrap();
        if probe.as_ref().is_some_and(|probe| &probe.kind == kind) {
            probe.take()
        } else {
            None
        }
    };
    if let Some(probe) = probe {
        probe.reached.notify_one();
        probe.resume.notified().await;
    }
}

fn configure() {
    crate::test_database::init();
    crate::config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name": LOCAL, "db":{"url":"unused"},
                "sender_workers":2,"max_concurrent_requests":2,
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
            }))
            .unwrap(),
        )
        .expect("run alone in an empty dedicated test process");
}

async fn execute(sql: &str) {
    diesel::sql_query(sql)
        .execute(&mut crate::data::db())
        .await
        .unwrap();
}

async fn receipt(sn: i64, room: &str, user: &str, event: &str, ty: &str, data: Value) {
    diesel::sql_query("INSERT INTO event_receipts (sn,ty,room_id,user_id,event_id,event_sn,json_data,receipt_at) VALUES ($1,$2,$3,$4,$5,$1,$6,0)")
        .bind::<BigInt,_>(sn).bind::<Text,_>(ty).bind::<Text,_>(room)
        .bind::<Text,_>(user).bind::<Text,_>(event).bind::<Json,_>(data)
        .execute(&mut crate::data::db()).await.unwrap();
}

fn decoded(events: &[EduBuf]) -> Vec<Value> {
    events
        .iter()
        .map(|event| serde_json::from_slice(event).unwrap())
        .collect()
}

#[tokio::test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
async fn review_outbound_windows_are_bounded_and_acknowledged_atomically() {
    configure();
    let server: &ServerName = REMOTE.try_into().unwrap();
    execute("INSERT INTO room_joined_servers (room_id,server_id,occur_sn) VALUES ('!a:selectors.example','recipient.example',1),('!b:selectors.example','recipient.example',1)").await;
    execute("INSERT INTO e2e_key_changes (user_id,room_id,occur_sn,changed_at) SELECT '@u'||lpad(i::text,4,'0')||':selectors.example:8448','!a:selectors.example',100,0 FROM generate_series(1,301) i").await;
    execute("INSERT INTO device_streams (id,user_id,device_id) SELECT 5000+i,'@u'||lpad(i::text,4,'0')||':selectors.example:8448','D' FROM generate_series(1,301) i").await;
    execute("INSERT INTO e2e_key_changes (user_id,room_id,occur_sn,changed_at) VALUES ('@remote:elsewhere.example','!a:selectors.example',100,0),('@hidden:selectors.example:8448','!hidden:selectors.example',100,0),('@future:selectors.example:8448','!a:selectors.example',201,0)").await;
    let mut position = data::sending::SelectorPosition::complete(99);
    let mut users = HashSet::new();
    for page in 0..4 {
        let (events, next) = select_edus_device_changes(server, &position, 200, SELECT_EDU_LIMIT)
            .await
            .unwrap();
        assert!(events.len() <= SELECT_EDU_LIMIT);
        for event in decoded(&events) {
            let user = event["content"]["user_id"].as_str().unwrap();
            assert!(
                users.insert(user.to_owned()),
                "a tied prefix must not repeat: {user}"
            );
            assert!(event["content"]["stream_id"].as_u64().unwrap() > 5000);
            let Edu::DeviceListUpdate(update) = serde_json::from_value(event).unwrap() else {
                panic!("device selector emitted another EDU type");
            };
            assert!(
                update.prev_id.is_empty(),
                "empty prev_id may be omitted on the wire"
            );
        }
        if page < 3 {
            assert_eq!(next.confirmed_through(), 99);
        }
        position = next;
    }
    assert_eq!(
        users.len(),
        301,
        "a tie larger than three transactions must finish"
    );
    assert_eq!(position, data::sending::SelectorPosition::complete(200));
    let (old_device, old_receipt) = data::sending::selector_positions(server, 100)
        .await
        .unwrap();
    assert_eq!(old_device, data::sending::SelectorPosition::complete(100));
    assert_eq!(old_receipt, old_device);
    assert!(
        select_edus_device_changes(server, &old_device, 200, SELECT_EDU_LIMIT)
            .await
            .unwrap()
            .0
            .is_empty(),
        "a legacy scalar cursor has consumed every user at its SN"
    );

    execute("INSERT INTO event_receipts (sn,ty,room_id,user_id,event_id,event_sn,json_data,receipt_at) SELECT 999+i,'m.read',CASE WHEN i%2=0 THEN '!a:selectors.example' ELSE '!b:selectors.example' END,'@r'||i||':selectors.example:8448','$r'||i,999+i,'{}',0 FROM generate_series(1,270) i").await;
    receipt(
        1270,
        "!a:selectors.example",
        "@private:selectors.example:8448",
        "$private",
        "m.read.private",
        json!({}),
    )
    .await;
    receipt(
        1271,
        "!a:selectors.example",
        "@remote:elsewhere.example",
        "$remote",
        "m.read",
        json!({}),
    )
    .await;
    receipt(
        1272,
        "!hidden:selectors.example",
        "@hidden:selectors.example:8448",
        "$hidden",
        "m.read",
        json!({}),
    )
    .await;
    receipt(
        1273,
        "!a:selectors.example",
        "@latest:selectors.example:8448",
        "$zzz",
        "m.read",
        json!({"ts":1}),
    )
    .await;
    receipt(
        1274,
        "!a:selectors.example",
        "@latest:selectors.example:8448",
        "$aaa",
        "m.read",
        json!({"ts":2}),
    )
    .await;
    receipt(
        1275,
        "!a:selectors.example",
        "@thread:selectors.example:8448",
        "$unthreaded",
        "m.read",
        json!({"ts":3}),
    )
    .await;
    receipt(
        1276,
        "!a:selectors.example",
        "@thread:selectors.example:8448",
        "$threaded",
        "m.read",
        json!({"ts":4,"thread_id":"main"}),
    )
    .await;
    receipt(
        2001,
        "!a:selectors.example",
        "@future:selectors.example:8448",
        "$future",
        "m.read",
        json!({}),
    )
    .await;
    let mut position = data::sending::SelectorPosition::complete(999);
    let mut received = Vec::new();
    for _ in 0..4 {
        let (event, next) = select_edus_receipts(server, &position, 2000).await.unwrap();
        if let Some(event) = event {
            let event: Value = serde_json::from_slice(&event).unwrap();
            let mut count = 0;
            for room in event["content"].as_object().unwrap().values() {
                for (user, receipt) in room["m.read"].as_object().unwrap() {
                    count += 1;
                    assert!(user.ends_with(LOCAL));
                    assert!(
                        !user.contains("private")
                            && !user.contains("hidden")
                            && !user.contains("future")
                    );
                    received.push(receipt.clone());
                }
            }
            assert!(
                count <= SELECT_RECEIPT_LIMIT,
                "receipt budget is global across rooms"
            );
        }
        assert!(next.sn >= position.sn);
        position = next;
        if position.sn == 2000 {
            break;
        }
    }
    assert_eq!(position.sn, 2000);
    assert_eq!(received.len(), 273);
    assert!(
        received
            .iter()
            .any(|r| r["event_ids"] == json!(["$aaa"]) && r["data"]["ts"] == 2)
    );
    assert!(!received.iter().any(|r| r["event_ids"] == json!(["$zzz"])));
    assert!(
        received
            .iter()
            .any(|r| r["event_ids"] == json!(["$unthreaded"]))
    );
    assert!(
        received
            .iter()
            .any(|r| r["event_ids"] == json!(["$threaded"]) && r["data"]["thread_id"] == "main")
    );

    // Inject a real DELETE failure after both selector positions and the
    // scalar/presence confirmation have executed in the same transaction.
    data::sending::initialize_edu_cursor(server, 99)
        .await
        .unwrap();
    let kind = OutgoingKind::Normal(server.to_owned());
    let id = super::super::queue_request(&kind, &SendingEventType::Flush)
        .await
        .unwrap();
    super::super::claim_queued_requests(&[(id, SendingEventType::Flush)])
        .await
        .unwrap();
    #[cfg(feature = "unstable-msc4495")]
    execute("INSERT INTO presence_recipient_sets (user_id,server_id,stream_id,recipients,pending_stream_id,pending_recipients,pending_edu_sn) VALUES ('@presence:selectors.example:8448','recipient.example',1,'[\"@old:recipient.example\"]',2,'[\"@new:recipient.example\"]',2000)").await;
    execute("CREATE FUNCTION review_reject_selector_ack() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected ACK failure'; END $$").await;
    execute("CREATE TRIGGER review_selector_ack BEFORE DELETE ON outgoing_requests FOR EACH ROW EXECUTE FUNCTION review_reject_selector_ack()").await;
    let window = EduWindow {
        events: vec![],
        confirmed_through: 100,
        presence_sn: Some(2000),
        device: data::sending::SelectorPosition {
            sn: 100,
            token: Some("@u0098:selectors.example:8448".into()),
        },
        receipt: data::sending::SelectorPosition::complete(2000),
    };
    assert!(acknowledge_destination(&kind, Some(&window)).await.is_err());
    assert_eq!(
        data::sending::get_edu_cursor(server).await.unwrap(),
        Some(99)
    );
    assert_eq!(
        data::sending::selector_positions(server, 99).await.unwrap(),
        (
            data::sending::SelectorPosition::complete(99),
            data::sending::SelectorPosition::complete(99)
        )
    );
    assert_eq!(
        super::super::active_requests_for(&kind)
            .await
            .unwrap()
            .len(),
        1
    );
    #[cfg(feature = "unstable-msc4495")]
    assert_eq!(
        crate::user::presence::recipients::sent_state(
            &UserId::parse(format!("@presence:{LOCAL}")).unwrap(),
            server
        )
        .await
        .unwrap()
        .pending
        .unwrap()
        .0,
        2
    );
    execute("DROP TRIGGER review_selector_ack ON outgoing_requests").await;
    acknowledge_destination(&kind, Some(&window)).await.unwrap();
    assert_eq!(
        data::sending::get_edu_cursor(server).await.unwrap(),
        Some(100)
    );
    assert_eq!(
        data::sending::selector_positions(server, 99).await.unwrap(),
        (window.device.clone(), window.receipt.clone())
    );
    assert!(
        super::super::active_requests_for(&kind)
            .await
            .unwrap()
            .is_empty()
    );
    let no_slots = select_edu_window(server, 0).await.unwrap();
    assert!(no_slots.events.is_empty());
    assert_eq!(no_slots.confirmed_through, 100);
    assert_eq!(no_slots.device, window.device);
    assert_eq!(no_slots.receipt, window.receipt);
    #[cfg(feature = "unstable-msc4495")]
    {
        let state = crate::user::presence::recipients::sent_state(
            &UserId::parse(format!("@presence:{LOCAL}")).unwrap(),
            server,
        )
        .await
        .unwrap();
        assert!(state.pending.is_none());
        assert_eq!(
            state.confirmed.unwrap().0,
            2,
            "presence confirmation uses its actual bound, not the common minimum"
        );
    }
    measure_selectors().await;
}

type BenchmarkOutput = (
    BTreeMap<OwnedUserId, i64>,
    BTreeMap<(OwnedRoomId, OwnedUserId, OwnedEventId), Value>,
);

async fn legacy_selectors(server: &ServerName) -> BenchmarkOutput {
    let mut devices = BTreeMap::new();
    let mut receipts = BTreeMap::new();
    for room in state::server_joined_rooms(server).await.unwrap() {
        for user in crate::room::keys_changed_users(&room, 0, Some(1_000_000))
            .await
            .unwrap()
        {
            if user.is_local() && !devices.contains_key(&user) {
                let stream = data::user::key::device_list_stream_id(&user).await.unwrap();
                devices.insert(user, stream);
            }
        }
    }
    for room in state::server_joined_rooms(server).await.unwrap() {
        for (user, content) in data::room::receipt::read_receipts(&room, 0).await.unwrap() {
            if !user.is_local() {
                continue;
            }
            for (event, types) in content.0 {
                if let Some(users) = types.get(&crate::core::events::receipt::ReceiptType::Read)
                    && let Some(receipt) = users.get(&user)
                {
                    receipts.insert(
                        (room.clone(), user.clone(), event),
                        serde_json::to_value(receipt).unwrap(),
                    );
                }
            }
        }
    }
    (devices, receipts)
}

async fn batched_selectors(server: &ServerName, local: &ServerName) -> BenchmarkOutput {
    let devices = data::sending::outgoing_device_changes(
        server,
        local,
        &data::sending::SelectorPosition::complete(0),
        1_000_000,
        SELECT_EDU_LIMIT + 1,
    )
    .await
    .unwrap();
    let receipts =
        data::sending::outgoing_receipts(server, local, 0, 1_000_000, SELECT_RECEIPT_LIMIT + 1)
            .await
            .unwrap();
    (
        devices
            .into_iter()
            .map(|row| (row.user_id, row.stream_id))
            .collect(),
        receipts
            .into_iter()
            .map(|row| ((row.room_id, row.user_id, row.event_id), row.json_data))
            .collect(),
    )
}

fn percentile(samples: &mut [Duration], percent: usize) -> u128 {
    samples.sort_unstable();
    samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)].as_micros()
}

async fn measure_selectors() {
    const SAMPLES: usize = 20;
    for rooms in [1, 16, 256] {
        execute(&format!("INSERT INTO room_joined_servers (room_id,server_id,occur_sn) SELECT '!bench{rooms}-'||i||':selectors.example','bench{rooms}.example',1 FROM generate_series(1,{rooms}) i")).await;
        execute(&format!("INSERT INTO e2e_key_changes (user_id,room_id,occur_sn,changed_at) SELECT '@bench'||u||':selectors.example:8448','!bench{rooms}-'||r||':selectors.example',10,0 FROM generate_series(1,{rooms}) r CROSS JOIN generate_series(1,16) u")).await;
        execute(&format!("INSERT INTO event_receipts (sn,ty,room_id,user_id,event_id,event_sn,json_data,receipt_at) SELECT 100000+{rooms}*1000+i,'m.read','!bench{rooms}-'||i||':selectors.example','@bench1:selectors.example:8448','$bench{rooms}-'||i,1,'{{}}',0 FROM generate_series(1,{rooms}) i")).await;
        let server = ServerName::parse(format!("bench{rooms}.example")).unwrap();
        let local: &ServerName = LOCAL.try_into().unwrap();
        let spy = SqlSpy::install().await;
        let mut old_times = Vec::new();
        let mut new_times = Vec::new();
        let mut old_count = 0;
        let mut new_count = 0;
        for sample in 0..SAMPLES {
            let mut old = None;
            let mut new = None;
            // Alternate order to avoid always granting one selector the warm cache.
            for is_legacy in if sample.is_multiple_of(2) {
                [true, false]
            } else {
                [false, true]
            } {
                spy.reset();
                let start = Instant::now();
                let result = if is_legacy {
                    legacy_selectors(&server).await
                } else {
                    batched_selectors(&server, local).await
                };
                let elapsed = start.elapsed();
                let queries = spy
                    .snapshot()
                    .iter()
                    .filter(|q| {
                        let q = q.split("-- binds").next().unwrap();
                        q.contains("room_joined_servers")
                            || q.contains("e2e_key_changes")
                            || q.contains("device_streams")
                            || q.contains("event_receipts")
                    })
                    .count();
                if is_legacy {
                    old_times.push(elapsed);
                    old_count += queries;
                    old = Some(result);
                } else {
                    new_times.push(elapsed);
                    new_count += queries;
                    new = Some(result);
                }
            }
            assert_eq!(
                old, new,
                "batching preserves users, stream IDs, room/event identities and receipt JSON"
            );
            let output = new.unwrap();
            assert_eq!(output.0.len(), 16);
            assert_eq!(output.1.len(), rooms);
        }
        assert_eq!(old_count / SAMPLES, 2 + 2 * rooms + 16);
        assert_eq!(new_count / SAMPLES, 2);
        eprintln!(
            "P02 selector comparison rooms={rooms} users=16 samples={SAMPLES} SQL old={} new={} latency_us old[p50={},p95={},p99={}] new[p50={},p95={},p99={}]",
            old_count / SAMPLES,
            new_count / SAMPLES,
            percentile(&mut old_times, 50),
            percentile(&mut old_times, 95),
            percentile(&mut old_times, 99),
            percentile(&mut new_times, 50),
            percentile(&mut new_times, 95),
            percentile(&mut new_times, 99)
        );
        spy.assert_coverage().await;
        spy.uninstall().await;
    }
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

async fn count(sql: &str) -> i64 {
    diesel::sql_query(sql)
        .get_result::<Count>(&mut crate::data::db())
        .await
        .unwrap()
        .count
}

async fn wait_attempt(sequence: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while count(&format!(
            "SELECT CASE WHEN is_called THEN 1 ELSE 0 END::bigint AS count FROM {sequence}"
        ))
        .await
            == 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the real PostgreSQL trigger must enter its blocked phase");
}

async fn wait_drained(kind: &OutgoingKind) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if super::super::active_requests_for(kind)
                .await
                .unwrap()
                .is_empty()
                && super::super::queued_requests(kind, 30)
                    .await
                    .unwrap()
                    .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a healthy destination must finish while the other destination remains SQL-blocked");
}

#[tokio::test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
async fn review_outbound_scheduler_preserves_wakeup_during_empty_probe() {
    configure();
    let kind = OutgoingKind::Appservice("empty-probe".into());
    let reached = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    *EMPTY_PROBE.lock().unwrap() = Some(EmptyProbe {
        kind: kind.clone(),
        reached: reached.clone(),
        resume: resume.clone(),
    });
    let (sender, receiver) = mpsc::channel(1);
    let worker = tokio::spawn(supervise(receiver, Duration::from_millis(20)));
    // Let the initial sweep finish before admitting the row below, so only
    // the explicit wakeup can deliver it before the next 30-second sweep.
    tokio::time::timeout(Duration::from_secs(3), INITIAL_SWEEP_FINISHED.notified())
        .await
        .unwrap();
    sender.send(kind.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), reached.notified())
        .await
        .unwrap();
    super::super::queue_request(&kind, &SendingEventType::Flush)
        .await
        .unwrap();
    sender.send(kind).await.unwrap();
    // The one-slot channel cannot grant this permit until the scheduler has
    // consumed the notification while the empty probe is still in flight.
    let consumed = tokio::time::timeout(Duration::from_secs(3), sender.reserve())
        .await
        .unwrap()
        .unwrap();
    resume.notify_one();
    tokio::time::timeout(Duration::from_secs(3), async {
        while count("SELECT count(*)::bigint AS count FROM outgoing_requests WHERE appservice_id='empty-probe'").await != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a wakeup during an empty probe must not wait for the periodic sweep");
    drop(consumed);
    worker.abort();
    let _ = worker.await;
}

#[tokio::test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
async fn review_outbound_scheduler_is_fair_under_slow_claim_and_ack() {
    configure();
    assert_eq!(destination_budget(), 2);
    execute("CREATE SEQUENCE review_slow_claim_attempts").await;
    execute("CREATE SEQUENCE review_slow_ack_attempts").await;
    execute("CREATE FUNCTION review_block_claim() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.appservice_id='slow-claim' AND NEW.state='pending' THEN PERFORM nextval('review_slow_claim_attempts'); PERFORM pg_advisory_xact_lock(7711); END IF; RETURN NEW; END $$").await;
    execute("CREATE TRIGGER review_slow_claim BEFORE UPDATE ON outgoing_requests FOR EACH ROW EXECUTE FUNCTION review_block_claim()").await;
    execute("CREATE FUNCTION review_block_ack() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.appservice_id='slow-ack' THEN PERFORM nextval('review_slow_ack_attempts'); PERFORM pg_advisory_xact_lock(7712); END IF; RETURN OLD; END $$").await;
    execute("CREATE TRIGGER review_slow_ack BEFORE DELETE ON outgoing_requests FOR EACH ROW EXECUTE FUNCTION review_block_ack()").await;
    let mut locks = data::checkout().await.unwrap();
    diesel::sql_query("SELECT pg_advisory_lock(7711),pg_advisory_lock(7712)")
        .execute(&mut locks)
        .await
        .unwrap();
    let (sender, receiver) = mpsc::channel(32);
    let worker = tokio::spawn(supervise(receiver, Duration::from_millis(20)));
    tokio::time::timeout(Duration::from_secs(3), async {
        while !is_ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let claim = OutgoingKind::Appservice("slow-claim".into());
    let ack = OutgoingKind::Appservice("slow-ack".into());
    let healthy = OutgoingKind::Appservice("healthy-first".into());
    let held = OutgoingKind::Appservice("healthy-budget-waiter".into());
    super::super::queue_request(&claim, &SendingEventType::Flush)
        .await
        .unwrap();
    sender.send(claim.clone()).await.unwrap();
    wait_attempt("review_slow_claim_attempts").await;
    for _ in 0..16 {
        sender.send(claim.clone()).await.unwrap();
    }
    super::super::queue_request(&healthy, &SendingEventType::Flush)
        .await
        .unwrap();
    let started = Instant::now();
    sender.send(healthy.clone()).await.unwrap();
    wait_drained(&healthy).await;
    let healthy_latency = started.elapsed();
    assert_eq!(count("SELECT count(*)::bigint AS count FROM pg_locks WHERE locktype='advisory' AND NOT granted AND objid=7711").await,1,"duplicate wakeups must not start concurrent work for one destination");

    super::super::queue_request(&ack, &SendingEventType::Flush)
        .await
        .unwrap();
    sender.send(ack.clone()).await.unwrap();
    wait_attempt("review_slow_ack_attempts").await;
    super::super::queue_request(&held, &SendingEventType::Flush)
        .await
        .unwrap();
    sender.send(held.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        super::super::queued_requests(&held, 30)
            .await
            .unwrap()
            .len(),
        1,
        "two blocked destination futures must saturate the configured two-slot budget"
    );
    assert!(
        super::super::active_requests_for(&held)
            .await
            .unwrap()
            .is_empty()
    );
    diesel::sql_query("SELECT pg_advisory_unlock(7711)")
        .execute(&mut locks)
        .await
        .unwrap();
    wait_drained(&claim).await;
    wait_drained(&held).await;
    assert_eq!(count("SELECT count(*)::bigint AS count FROM pg_locks WHERE locktype='advisory' AND NOT granted AND objid=7712").await,1,"healthy claim and ACK must finish while the other ACK remains blocked");
    eprintln!(
        "P02 scheduler budget=2 blocked_claims=1 healthy_completion_us={} saturation_waiter_preserved=true",
        healthy_latency.as_micros()
    );

    // Cancellation must leave the unacknowledged row recoverable; restart without a wakeup.
    worker.abort();
    let _ = worker.await;
    diesel::sql_query("SELECT pg_advisory_unlock(7712)")
        .execute(&mut locks)
        .await
        .unwrap();
    drop(locks);
    execute("DROP TRIGGER review_slow_ack ON outgoing_requests").await;
    assert_eq!(
        super::super::active_requests_for(&ack).await.unwrap().len(),
        1,
        "cancelling the blocked ACK must retain its unacknowledged row"
    );
    let (_sender, receiver) = mpsc::channel(1);
    let restarted = tokio::spawn(supervise(receiver, Duration::from_millis(20)));
    wait_drained(&ack).await;
    restarted.abort();
    let _ = restarted.await;
    assert!(!is_running());

    // Cancel before a BEGIN batch completes, so Diesel has not yet increased
    // its recorded transaction depth. The batch holds a real table lock;
    // cleanup must not depend on checking the cancelled object out again.
    let mut begin_lock = data::checkout().await.unwrap();
    diesel::sql_query("SELECT pg_advisory_lock(7713)")
        .execute(&mut begin_lock)
        .await
        .unwrap();
    let beginning = tokio::spawn(async {
        let mut connection = TransactionConnection::new(data::checkout().await.unwrap());
        diesel_async::AnsiTransactionManager::begin_transaction_sql(
            connection.connection(),
            "BEGIN; LOCK TABLE outgoing_requests IN ACCESS EXCLUSIVE MODE; SELECT pg_advisory_xact_lock(7713)",
        ).await.unwrap();
        panic!("the test must cancel the blocked BEGIN before it returns");
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while count("SELECT count(*)::bigint AS count FROM pg_locks WHERE locktype='advisory' AND NOT granted AND objid=7713").await != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("BEGIN batch must be blocked before cancellation");
    beginning.abort();
    let _ = beginning.await;
    diesel::sql_query("SELECT pg_advisory_unlock(7713)")
        .execute(&mut begin_lock)
        .await
        .unwrap();
    drop(begin_lock);
    execute("CREATE INDEX review_begin_cleanup ON outgoing_requests (id)").await;
}
