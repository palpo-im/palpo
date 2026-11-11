use std::time::Duration;

use diesel_async::{AsyncConnection, SimpleAsyncConnection};
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

const LOCAL: &str = "observability-review.example";
const PROCESS_CASE: &str =
    "observability::tests::review_outbound_process_restart_preserves_requests_and_reports_recovery";

fn configure() {
    crate::config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":LOCAL,"db":{"url":"unused-secret-diagnostics-test"},
                "auto_join_rooms":[],"ip_range_denylist":[],"presence":{"enable":false}
            }))
            .unwrap(),
        )
        .unwrap();
}

fn run_async(work: impl std::future::Future<Output = ()> + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(work)
        })
        .unwrap()
        .join()
        .unwrap();
}

#[tokio::test]
async fn review_diagnostics_without_initialized_pools_does_not_fabricate_counts() {
    let diagnostics = check().await;
    assert_eq!(diagnostics.queue.state, QueueSampleState::NotInitialized);
    assert!(diagnostics.queue.snapshot.is_none());
    assert!(!diagnostics.readiness.ready);
    assert_eq!(diagnostics.outbound.len(), 3);
    assert_eq!(diagnostics.outcome_names.len(), 8);
}

#[test]
fn review_send_outcomes_include_cancellation_with_bounded_labels() {
    let kind = crate::sending::OutgoingKind::Appservice("never-export-this-destination".into());
    let before = SENDS[1].latency.snapshot();
    drop(SendAttempt::start(&kind));
    SendAttempt::start(&kind).finish(None);
    SendAttempt::start(&kind).finish(Some(&AppError::public("never-export-this-error")));
    let after = SENDS[1].latency.snapshot();
    assert!(after.cancelled > before.cancelled);
    assert!(after.completed > before.completed);
    assert!(after.failed > before.failed);
    assert_eq!(
        OUTCOME_NAMES,
        [
            "success",
            "storage",
            "transport_timeout",
            "transport",
            "decode",
            "matrix",
            "application",
            "cancelled"
        ]
    );
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_outbound_diagnostics_tracks_queue_pool_failures_and_restored_rows() {
    run_async(async {
        crate::test_database::init();
        configure();
        // Roll back the complete down/up drill: exercise a genuine legacy row,
        // without changing the migration ledger or leaving test data behind.
        let mut connection = data::checkout().await.unwrap();
        let result = connection.transaction::<(), AppError, _>(async |conn| {
            conn.batch_execute(include_str!("../../data/migrations/2026-11-10-010000_outgoing_enqueue_time/down.sql")).await?;
            conn.batch_execute("INSERT INTO outgoing_requests(kind,appservice_id,state) VALUES ('appservice','legacy','created')").await?;
            conn.batch_execute(include_str!("../../data/migrations/2026-11-10-010000_outgoing_enqueue_time/up.sql")).await?;
            conn.batch_execute("INSERT INTO outgoing_requests(kind,appservice_id,state) VALUES ('appservice','new','created')").await?;
            let snapshot = diesel::sql_query(QUEUE_SQL).get_result::<QueueSnapshot>(conn).await?;
            assert_eq!((snapshot.total_rows, snapshot.unknown_age_rows), (2, 1));
            assert!(snapshot.oldest_known_age_ms.is_some());
            Err(AppError::public("rollback migration drill"))
        }).await;
        assert!(result.is_err());
        drop(connection);

        let sampler = QueueSampler::new();
        let empty = sampler.sample(Duration::ZERO).await.snapshot.unwrap();
        assert_eq!(
            (
                empty.total_rows,
                empty.unknown_age_rows,
                empty.oldest_known_age_ms
            ),
            (0, 0, None)
        );
        let mut connection = data::checkout().await.unwrap();
        connection.batch_execute("INSERT INTO outgoing_requests(kind,appservice_id,state,retry_count,enqueued_at) VALUES
            ('appservice','ops-a','created',0,floor(extract(epoch FROM clock_timestamp())*1000)::bigint-10000),
            ('appservice','ops-b','pending',2,floor(extract(epoch FROM clock_timestamp())*1000)::bigint-20000),
            ('appservice','ops-c','created',0,NULL),
            ('appservice','ops-d','created',0,floor(extract(epoch FROM clock_timestamp())*1000)::bigint+5000)").await.unwrap();
        drop(connection);
        let fresh = sampler.sample(Duration::ZERO).await;
        assert_eq!(fresh.state, QueueSampleState::Fresh);
        let snapshot = fresh.snapshot.unwrap();
        assert_eq!(
            (
                snapshot.total_rows,
                snapshot.queued_rows,
                snapshot.claimed_rows,
                snapshot.retry_marked_rows,
                snapshot.unknown_age_rows,
                snapshot.future_timestamp_rows
            ),
            (4, 3, 1, 1, 1, 1)
        );
        assert!(snapshot.oldest_known_age_ms.unwrap() >= 20_000);
        assert_eq!(
            sampler.sample(QUEUE_CACHE_TTL).await.state,
            QueueSampleState::Cached
        );

        // Schema unavailability is a query fault, not a simulated network outage.
        data::checkout()
            .await
            .unwrap()
            .batch_execute("ALTER TABLE outgoing_requests RENAME TO unavailable_ops_requests")
            .await
            .unwrap();
        let failed = sampler.sample(Duration::ZERO).await;
        assert_eq!(failed.state, QueueSampleState::Failed);
        assert_eq!(failed.snapshot.unwrap().total_rows, 4);
        assert_eq!(
            sampler.sample(QUEUE_CACHE_TTL).await.state,
            QueueSampleState::Stale
        );
        data::checkout()
            .await
            .unwrap()
            .batch_execute("ALTER TABLE unavailable_ops_requests RENAME TO outgoing_requests")
            .await
            .unwrap();
        assert_eq!(
            sampler.sample(Duration::ZERO).await.state,
            QueueSampleState::Fresh
        );

        // Real PostgreSQL session termination, followed by a successful fresh
        // checkout. This does not claim that PostgreSQL itself was restarted.
        #[derive(diesel::QueryableByName)]
        struct Pid {
            #[diesel(sql_type = diesel::sql_types::Integer)]
            pid: i32,
        }
        let mut victim = data::checkout().await.unwrap();
        let pid = diesel::sql_query("SELECT pg_backend_pid() AS pid")
            .get_result::<Pid>(&mut victim)
            .await
            .unwrap()
            .pid;
        let mut control = data::coordination_connect();
        diesel::sql_query("SELECT pg_terminate_backend($1)")
            .bind::<diesel::sql_types::Integer, _>(pid)
            .execute(&mut control)
            .await
            .unwrap();
        assert!(victim.batch_execute("SELECT 1").await.is_err());
        drop(victim);
        drop(control);
        data::checkout()
            .await
            .unwrap()
            .batch_execute("SELECT 1")
            .await
            .unwrap();

        let before = data::telemetry::QUERY_POOL_WAIT.snapshot();
        let mut held = Vec::new();
        for _ in 0..data::status().max_size {
            held.push(data::checkout().await.unwrap());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), data::checkout())
                .await
                .is_err()
        );
        let timeout = sampler.sample(Duration::ZERO).await;
        assert_eq!(timeout.state, QueueSampleState::TimedOut);
        assert_eq!(timeout.snapshot.unwrap().total_rows, 4);
        assert!(data::telemetry::QUERY_POOL_WAIT.snapshot().cancelled >= before.cancelled + 2);
        drop(held);
        assert_eq!(
            sampler.sample(Duration::ZERO).await.state,
            QueueSampleState::Fresh
        );

        let semaphore = Semaphore::new(0);
        let before = PERMIT_WAIT.snapshot();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                acquire_outbound_permit(&semaphore)
            )
            .await
            .is_err()
        );
        semaphore.add_permits(1);
        drop(acquire_outbound_permit(&semaphore).await.unwrap());
        semaphore.close();
        assert!(acquire_outbound_permit(&semaphore).await.is_err());
        let after = PERMIT_WAIT.snapshot();
        assert_eq!(
            (
                after.completed - before.completed,
                after.failed - before.failed,
                after.cancelled - before.cancelled
            ),
            (1, 1, 1)
        );

        let service = Service::new(crate::routing::root());
        let mut response = TestClient::get("http://localhost/diagnostics")
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let body: Value = response.take_json().await.unwrap();
        assert_eq!(body["queue"]["snapshot"]["total_rows"], 4);
        assert_eq!(body["readiness"]["ready"], false);
        assert!(!body.to_string().contains("unused-secret"));
        let response = TestClient::get("http://localhost/ready")
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::SERVICE_UNAVAILABLE));
        println!(
            "M01 queue drill: empty=0, backlog=4/queued3/claimed1/retry1/unknown1; schema failure retains4; pool timeout retains4; backend termination reconnects; restored=4"
        );
    });
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; spawns and kills child processes; run alone"]
fn review_outbound_process_restart_preserves_requests_and_reports_recovery() {
    if let Ok(phase) = std::env::var("PALPO_OPS_CHILD_PHASE") {
        run_async(async move { restart_child(&phase).await });
        return;
    }
    restart_parent();
}

fn checkpoint(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("PALPO_OPS_CHECKPOINT_DIR").unwrap()).join(name)
}

async fn restart_child(phase: &str) {
    if phase == "spool" {
        crate::test_database::init();
    } else {
        let config = serde_json::from_value(json!({"url":std::env::var("PALPO_TEST_DATABASE_URL").unwrap(),"pool_size":10,"statement_timeout":5000})).unwrap();
        data::init(&config).unwrap();
    }
    configure();
    if phase == "spool" {
        use crate::core::{OwnedDeviceId, UnixMillis, UserId};
        let owner = UserId::parse(format!("@ops:{LOCAL}")).unwrap();
        data::user::create_user(&data::user::NewDbUser {
            id: owner.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: "ops".into(),
            server_name: owner.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        data::user::ensure_profile_exists(&owner).await.unwrap();
        data::user::device::create_device(
            &owner,
            &OwnedDeviceId::from("OPS"),
            "ops-test-token",
            None,
            None,
        )
        .await
        .unwrap();
        crate::user::get_push_rules(&owner).await.unwrap();
        // A genuine stored room event ensures both processes perform actual
        // appservice HTTP delivery rather than ACKing an empty Flush payload.
        let service = Service::new(crate::routing::root());
        let router = service.router();
        let created: Value = tokio::spawn(async move {
            let mut response = TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .add_header("authorization", "Bearer ops-test-token", true)
                .json(&json!({"room_version":"11","preset":"private_chat"}))
                .send(router)
                .await;
            assert_eq!(response.status_code, Some(StatusCode::OK));
            response.take_json().await.unwrap()
        })
        .await
        .unwrap();
        #[derive(diesel::QueryableByName)]
        struct Event {
            #[diesel(sql_type = diesel::sql_types::Text)]
            id: String,
        }
        let event =
            diesel::sql_query("SELECT id FROM events WHERE room_id=$1 AND ty='m.room.create'")
                .bind::<diesel::sql_types::Text, _>(created["room_id"].as_str().unwrap())
                .get_result::<Event>(&mut crate::data::db())
                .await
                .unwrap();
        crate::appservice::register_appservice(serde_json::from_value(json!({"id":"ops-restart","url":std::env::var("PALPO_OPS_PEER_URL").unwrap(),"as_token":"ops-as-token","hs_token":"ops-hs-token","sender_localpart":"bridge","namespaces":{}})).unwrap()).await.unwrap();
        let mut connection = data::connect();
        for _ in 0..3 {
            diesel::sql_query("INSERT INTO outgoing_requests(kind,appservice_id,pdu_id,state) VALUES ('appservice','ops-restart',$1,'pending')").bind::<diesel::sql_types::Text,_>(&event.id).execute(&mut connection).await.unwrap();
        }
        drop(connection);
        std::fs::write(
            checkpoint("before.json"),
            serde_json::to_vec(&queue_ids().await).unwrap(),
        )
        .unwrap();
    } else {
        let expected: Vec<i64> =
            serde_json::from_slice(&std::fs::read(checkpoint("before.json")).unwrap()).unwrap();
        assert_eq!(
            queue_ids().await,
            expected,
            "the killed process must leave every unacknowledged durable row"
        );
        assert_eq!(
            worker_snapshot().completed_recoveries,
            0,
            "process counters must start fresh"
        );
    }
    crate::sending::guard::init().start();
    crate::readiness::mark_startup_complete();
    if phase == "spool" {
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                // HTTP 503 is followed by a retry held open at the gateway.
                if AtomicU64::load(&SENDS[1].outcomes[6], Ordering::Relaxed) > 0
                    && AtomicU64::load(&SENDS[1].active, Ordering::Relaxed) > 0
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("HTTP 503 must become an observable durable retry");
        let fault = check().await;
        assert_eq!(fault.queue.snapshot.as_ref().unwrap().total_rows, 3);
        assert_eq!(fault.queue.snapshot.as_ref().unwrap().retry_marked_rows, 3);
        assert_eq!(fault.outbound[1].active_attempts, 1);
        assert!(fault.outbound[1].outcome_counts[6] >= 1);
        assert!(fault.worker.heartbeat_age_ms.unwrap() < 5_000);
        std::fs::write(
            checkpoint("fault.json"),
            serde_json::to_vec(&fault).unwrap(),
        )
        .unwrap();
        std::future::pending::<()>().await;
    }
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if queue_ids().await.is_empty() && crate::sending::guard::is_ready() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("restarted real worker must send and ACK all preserved rows");
    let diagnostics = check().await;
    assert!(diagnostics.worker.completed_recoveries >= 1);
    assert!(diagnostics.worker.last_recovery_ms.is_some());
    assert!(diagnostics.worker.heartbeat_age_ms.unwrap() < 5_000);
    assert_eq!(diagnostics.queue.snapshot.as_ref().unwrap().total_rows, 0);
    assert!(diagnostics.outbound[1].outcome_counts[0] >= 1);
    std::fs::write(
        checkpoint("after.json"),
        serde_json::to_vec(&diagnostics).unwrap(),
    )
    .unwrap();
}

async fn queue_ids() -> Vec<i64> {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        id: i64,
    }
    diesel::sql_query("SELECT id FROM outgoing_requests ORDER BY id")
        .load::<Row>(&mut crate::data::db())
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.id)
        .collect()
}

// A failed assertion must never leave an orphaned spool/recovery server.
struct ChildProcess(std::process::Child);

impl std::ops::Deref for ChildProcess {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ChildProcess {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

struct PeerCleanup {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    restored: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for PeerCleanup {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.restored.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn restart_parent() {
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let directory = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let peer_url = format!("http://{}", listener.local_addr().unwrap());
    let restored = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let peer = {
        let restored = restored.clone();
        let stop = stop.clone();
        let requests = requests.clone();
        std::thread::spawn(move || {
            while !AtomicBool::load(&stop, Ordering::Acquire) {
                let (mut socket, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                let (head_end, length) = loop {
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&bytes[..end]);
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while bytes.len() < head_end + length {
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let number = {
                    let mut requests = requests.lock().unwrap();
                    requests
                        .push(serde_json::from_slice(&bytes[head_end..head_end + length]).unwrap());
                    requests.len()
                };
                if number == 1 {
                    socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
                    continue;
                }
                while !AtomicBool::load(&restored, Ordering::Acquire)
                    && !AtomicBool::load(&stop, Ordering::Acquire)
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                let _ = socket.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                );
            }
        })
    };
    let mut peer_cleanup = PeerCleanup {
        stop: stop.clone(),
        restored: restored.clone(),
        thread: Some(peer),
    };
    let spawn = |phase: &str| {
        let log = std::fs::File::create(directory.path().join(format!("{phase}.log"))).unwrap();
        ChildProcess(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    PROCESS_CASE,
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PALPO_OPS_CHILD_PHASE", phase)
                .env("PALPO_OPS_CHECKPOINT_DIR", directory.path())
                .env("PALPO_OPS_PEER_URL", &peer_url)
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap(),
        )
    };
    let mut first = spawn("spool");
    let deadline = Instant::now() + Duration::from_secs(60);
    while requests.lock().unwrap().len() < 2 || !directory.path().join("fault.json").exists() {
        if let Some(status) = first.try_wait().unwrap() {
            panic!(
                "spool child failed {status}: {}",
                std::fs::read_to_string(directory.path().join("spool.log")).unwrap()
            );
        }
        assert!(
            Instant::now() < deadline,
            "no in-flight HTTP request: {}",
            std::fs::read_to_string(directory.path().join("spool.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // The gateway has received all three events but has not returned an ACK.
    first.kill().unwrap();
    first.wait().unwrap();
    restored.store(true, Ordering::Release);
    let mut second = spawn("recover");
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            second.kill().unwrap();
            panic!("recovery child timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    stop.store(true, Ordering::Release);
    peer_cleanup.thread.take().unwrap().join().unwrap();
    assert!(
        status.success(),
        "recovery child failed: {}",
        std::fs::read_to_string(directory.path().join("recover.log")).unwrap()
    );
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "one refused, one unacknowledged retry and one recovered transaction"
    );
    assert_eq!(requests[0]["events"].as_array().unwrap().len(), 3);
    for attempts in requests.windows(2) {
        let mut stable = Vec::new();
        for request in attempts {
            let mut body = request.clone();
            for event in body["events"].as_array_mut().unwrap() {
                event["unsigned"].as_object_mut().unwrap().remove("age");
            }
            stable.push(body);
        }
        assert_eq!(
            stable[0], stable[1],
            "retry must preserve stable event content and IDs"
        );
        for (before, after) in attempts[0]["events"]
            .as_array()
            .unwrap()
            .iter()
            .zip(attempts[1]["events"].as_array().unwrap())
        {
            assert!(
                after["unsigned"]["age"].as_u64().unwrap()
                    >= before["unsigned"]["age"].as_u64().unwrap(),
                "event age must increase across retries and process restart"
            );
        }
    }
    let before: Vec<i64> =
        serde_json::from_slice(&std::fs::read(directory.path().join("before.json")).unwrap())
            .unwrap();
    assert_eq!(before.len(), 3);
    let after: Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("after.json")).unwrap())
            .unwrap();
    let fault: Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("fault.json")).unwrap())
            .unwrap();
    println!(
        "M01 real process kill/restart: durable rows before={before:?}; fault queue={}; fault active={}; fault outcomes={}; after queue={}; gateway transactions={}; worker={}",
        fault["queue"],
        fault["outbound"][1]["active_attempts"],
        fault["outbound"][1]["outcome_counts"],
        after["queue"],
        requests.len(),
        after["worker"]
    );
}
