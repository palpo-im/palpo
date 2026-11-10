//! Isolated real-HTTP/PG coverage for the operational readiness boundary.
use diesel_async::RunQueryDsl;
use salvo::http::StatusCode;
use salvo::prelude::Service;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

async fn status(service: &Service, expected: StatusCode) -> Value {
    let mut response = TestClient::get("http://localhost/ready")
        .send(service)
        .await;
    assert_eq!(response.headers()["cache-control"], "no-store");
    let actual = response.status_code;
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(actual, Some(expected), "{body}");
    body
}

async fn live(service: &Service) {
    for route in ["health", "healthz"] {
        let mut response = TestClient::get(format!("http://localhost/{route}"))
            .send(service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        assert_eq!(response.take_string().await.unwrap(), "ok");
    }
}

async fn wait_ready(service: &Service) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !is_ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(status(service, StatusCode::OK).await["ready"], true);
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
// The shared PostgreSQL batch selects the substring `database_`. This case
// owns process globals and must only run in the isolated exact-name batch.
fn review_readiness_http_tracks_pool_and_sender_recovery_without_breaking_liveness() {
    std::thread::Builder::new()
        .name("readiness-review-http".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run())
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn run() {
    crate::config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"readiness.example", "db":{"url":"unused-test-config"}
            }))
            .unwrap(),
        )
        .unwrap();
    let service = Service::new(crate::routing::root());
    let initial = status(&service, StatusCode::SERVICE_UNAVAILABLE).await;
    assert_eq!(initial["database"], "not_initialized");
    assert_eq!(initial["startup_complete"], false);
    assert_eq!(initial["outbound_worker"]["ready"], false);
    live(&service).await;

    crate::test_database::init();
    let initialized = status(&service, StatusCode::SERVICE_UNAVAILABLE).await;
    assert_eq!(initialized["database"], "ready");
    assert_eq!(initialized["startup_complete"], false);
    crate::readiness::mark_startup_complete();
    let stopped = status(&service, StatusCode::SERVICE_UNAVAILABLE).await;
    assert_eq!(stopped["startup_complete"], true);
    assert_eq!(stopped["outbound_worker"]["running"], false);

    let (_sender, receiver) = mpsc::channel(4);
    let worker = tokio::spawn(supervise(receiver, Duration::from_millis(20)));
    wait_ready(&service).await;
    live(&service).await;
    let public = crate::readiness::check().await;
    assert!(public.ready);
    assert!(public.database_pools.unwrap().query.max_size > 0);

    // Exhaust the real query pool. The combined checkout+query deadline must
    // make HTTP readiness fail promptly even though pool timeout is longer.
    let mut held = Vec::new();
    for _ in 0..data::status().max_size {
        held.push(data::connect().await.unwrap());
    }
    let timed_out = tokio::time::timeout(
        Duration::from_secs(3),
        status(&service, StatusCode::SERVICE_UNAVAILABLE),
    )
    .await
    .unwrap();
    assert_eq!(timed_out["database"], "timed_out");
    live(&service).await;
    drop(held);
    wait_ready(&service).await;

    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    let stopped = status(&service, StatusCode::SERVICE_UNAVAILABLE).await;
    assert_eq!(stopped["database"], "ready");
    assert_eq!(stopped["outbound_worker"]["running"], false);
    assert_eq!(stopped["outbound_worker"]["ready"], false);
    live(&service).await;

    // A sender task is not ready merely because it is alive: startup must
    // recover its durable queue. Break that table while SELECT 1 still works.
    diesel::sql_query("ALTER TABLE outgoing_requests RENAME TO readiness_unavailable_outgoing")
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    let restarts = restart_count();
    let (_sender, receiver) = mpsc::channel(4);
    let worker = tokio::spawn(supervise(receiver, Duration::from_millis(20)));
    tokio::time::timeout(Duration::from_secs(5), async {
        while restart_count() == restarts {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let recovering = status(&service, StatusCode::SERVICE_UNAVAILABLE).await;
    assert_eq!(recovering["database"], "ready");
    assert_eq!(recovering["outbound_worker"]["ready"], false);
    assert!(
        recovering["outbound_worker"]["restart_count"]
            .as_u64()
            .unwrap()
            > restarts
    );
    live(&service).await;
    diesel::sql_query("ALTER TABLE readiness_unavailable_outgoing RENAME TO outgoing_requests")
        .execute(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    wait_ready(&service).await;
    worker.abort();
    let _ = worker.await;
    assert!(!is_running());
    assert!(!is_ready());
}
