use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use super::*;

#[test]
fn review_preview_simultaneous_last_lease_drops_reclaim_every_key() {
    let requests = PreviewRequests::new(4);
    let barrier = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                for iteration in 0..1000 {
                    let lease = requests.try_enter(&format!("same?{iteration}")).unwrap();
                    barrier.wait();
                    drop(lease);
                    barrier.wait();
                    assert!(requests.locks.lock().unwrap().is_empty());
                    assert_eq!(requests.slots.available_permits(), 4);
                    barrier.wait();
                }
            });
        }
    });
}

#[tokio::test]
async fn review_preview_admission_bounds_keys_and_reclaims_waiters_on_cancel() {
    let requests = Arc::new(PreviewRequests::new(3));
    let first = requests.try_enter("same").unwrap();
    let lock = first.mutex.lock().await;
    let waiter_requests = requests.clone();
    let (registered, registration) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn(async move {
        let lease = waiter_requests.try_enter("same").unwrap();
        registered.send(()).unwrap();
        let _lock = lease.mutex.lock().await;
    });
    registration.await.unwrap();
    let other = requests.try_enter("other").unwrap();
    assert!(requests.try_enter("overflow").is_err());
    assert_eq!(requests.locks.lock().unwrap().len(), 2);
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    // A canceled waiter must not remove the holder's lock. New waiters must
    // still use exactly the original lock.
    let next = requests.try_enter("same").unwrap();
    assert!(Arc::ptr_eq(&first.mutex, &next.mutex));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), next.mutex.lock())
            .await
            .is_err()
    );
    drop(next);
    drop(other);
    drop(lock);
    drop(first);
    assert!(requests.locks.lock().unwrap().is_empty());
    assert_eq!(requests.slots.available_permits(), 3);
}

#[tokio::test]
async fn review_preview_key_is_not_replaced_while_waiters_or_holders_remain() {
    let requests = Arc::new(PreviewRequests::new(16));
    let current = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let requests = requests.clone();
        let current = current.clone();
        let maximum = maximum.clone();
        tasks.push(tokio::spawn(async move {
            let lease = requests.try_enter("same").unwrap();
            let _lock = lease.mutex.lock().await;
            let active = current.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(1)).await;
            current.fetch_sub(1, Ordering::SeqCst);
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
    assert!(requests.locks.lock().unwrap().is_empty());
    for index in 0..10_000 {
        let lease = requests.try_enter(&format!("failed?{index}")).unwrap();
        drop(lease);
    }
    assert!(requests.locks.lock().unwrap().is_empty());
    assert_eq!(requests.slots.available_permits(), 16);
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_preview_requests_reclaim_failed_urls_and_deduplicate() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(serde_json::json!({
                "server_name": "preview.example", "db": {"url": "unused"},
                "ip_range_denylist": [],
                "url_preview": {"domain_explicit_allowlist": ["127.0.0.1"]}
            }))
            .unwrap(),
        )
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let slow_started = Arc::new(Notify::new());
    let release_slow = Arc::new(Notify::new());
    let peer_hits = hits.clone();
    let peer_started = slow_started.clone();
    let peer_release = release_slow.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let hits = peer_hits.clone();
            let started = peer_started.clone();
            let release = peer_release.clone();
            tokio::spawn(async move {
                let mut headers = Vec::new();
                let mut chunk = [0; 1024];
                while !headers.windows(4).any(|w| w == b"\r\n\r\n") {
                    let read = stream.read(&mut chunk).await.unwrap();
                    if read == 0 {
                        return;
                    }
                    headers.extend_from_slice(&chunk[..read]);
                    assert!(headers.len() <= 8192);
                }
                hits.fetch_add(1, Ordering::SeqCst);
                let text = String::from_utf8(headers).unwrap();
                if text.starts_with("GET /slow ") {
                    started.notify_one();
                    release.notified().await;
                }
                let (kind, body) = if text.starts_with("GET /ok ") {
                    (
                        "text/html",
                        "<html><head><title>shared preview</title></head></html>",
                    )
                } else {
                    ("application/octet-stream", "unsupported")
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });

    // Exercise the production DB/cache/network path with unique failed URLs.
    for index in 0..128 {
        let url = Url::parse(&format!("{base}/failed?{index}")).unwrap();
        assert!(url_preview_allowed(&url));
        assert!(get_url_preview(&url).await.is_err());
        assert!(PREVIEW_REQUESTS.locks.lock().unwrap().is_empty());
    }
    assert_eq!(hits.load(Ordering::SeqCst), 128);

    let before = hits.load(Ordering::SeqCst);
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let url = Url::parse(&format!("{base}/ok")).unwrap();
        tasks.push(tokio::spawn(
            async move { get_url_preview(&url).await.unwrap() },
        ));
    }
    for task in tasks {
        assert_eq!(
            task.await.unwrap().og_title.as_deref(),
            Some("shared preview")
        );
    }
    // The existing spider makes a type probe followed by an HTML GET. Other
    // callers must observe the cache written by that single preview producer.
    assert_eq!(hits.load(Ordering::SeqCst) - before, 2);
    assert!(PREVIEW_REQUESTS.locks.lock().unwrap().is_empty());

    let url = Url::parse(&format!("{base}/slow")).unwrap();
    let slow = tokio::spawn(async move { get_url_preview(&url).await });
    tokio::time::timeout(Duration::from_secs(5), slow_started.notified())
        .await
        .unwrap();
    slow.abort();
    assert!(slow.await.unwrap_err().is_cancelled());
    assert!(PREVIEW_REQUESTS.locks.lock().unwrap().is_empty());
    assert_eq!(
        PREVIEW_REQUESTS.slots.available_permits(),
        MAX_PREVIEW_REQUESTS
    );
    release_slow.notify_one();
    server.abort();
    let _ = server.await;
}
