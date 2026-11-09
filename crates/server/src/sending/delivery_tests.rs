//! Real HTTP/DB push retry regression; run alone on an empty dedicated database.
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;
use crate::data::user::{NewDbAccessToken, NewDbUser, NewDbUserDevice};

async fn request(service: &Service, request: RequestBuilder, token: &str) -> Value {
    let request = request.add_header("Authorization", format!("Bearer {token}"), true);
    let router = service.router();
    let mut response = tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap();
    let body = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

async fn retry_push() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"push-retry.example", "db":{"url":"unused"},
                "ip_range_denylist":[], "max_concurrent_requests":1,
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
            }))
            .unwrap(),
        )
        .unwrap();
    let guard = guard::init();
    for name in ["alice", "bob"] {
        let user = UserId::parse(format!("@{name}:push-retry.example")).unwrap();
        let mut conn = connect().await.unwrap();
        diesel::insert_into(users::table)
            .values(NewDbUser {
                id: user.clone(),
                ty: None,
                is_admin: true,
                is_guest: false,
                is_local: true,
                localpart: name.into(),
                server_name: user.server_name().to_owned(),
                appservice_id: None,
                created_at: UnixMillis::now(),
            })
            .execute(&mut conn)
            .await
            .unwrap();
        diesel::insert_into(user_devices::table)
            .values(NewDbUserDevice {
                user_id: user.clone(),
                device_id: "D".into(),
                display_name: None,
                user_agent: None,
                is_hidden: false,
                last_seen_ip: None,
                last_seen_at: None,
                created_at: UnixMillis::now(),
            })
            .execute(&mut conn)
            .await
            .unwrap();
        diesel::insert_into(user_access_tokens::table)
            .values(NewDbAccessToken::new(user, "D".into(), name.into(), None))
            .execute(&mut conn)
            .await
            .unwrap();
    }
    let service = Service::new(crate::routing::root());
    let room = request(
        &service,
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .json(&json!({"preset":"public_chat","room_version":"11"})),
        "alice",
    )
    .await;
    let room = room["room_id"].as_str().unwrap();
    request(
        &service,
        TestClient::post(format!("http://localhost/_matrix/client/v3/join/{room}"))
            .json(&json!({})),
        "bob",
    )
    .await;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://{}/_matrix/push/v1/notify",
        listener.local_addr().unwrap()
    );
    let attempts = Arc::new(AtomicUsize::new(0));
    let received = attempts.clone();
    let peer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(45);
        for status in ["503 Service Unavailable", "200 OK"] {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "gateway did not receive retry");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 1024];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        let body: Value =
                            serde_json::from_slice(&request[end + 4..end + 4 + length]).unwrap();
                        assert_eq!(body["notification"]["devices"][0]["pushkey"], "retry-key");
                        break;
                    }
                }
            }
            let body = r#"{"rejected":[]}"#;
            write!(stream,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            received.fetch_add(1, Ordering::SeqCst);
        }
    });
    request(&service,TestClient::post("http://localhost/_matrix/client/v3/pushers/set").json(&json!({
        "kind":"http","app_id":"retry-app","app_display_name":"test","device_display_name":"test",
        "pushkey":"retry-key","lang":"en","data":{"url":url,"format":"event_id_only"}
    })),"bob").await;
    let sent = request(
        &service,
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/notify"
        ))
        .json(&json!({"msgtype":"m.text","body":"retry this notification"})),
        "alice",
    )
    .await;
    let event = EventId::parse(sent["event_id"].as_str().unwrap()).unwrap();
    let kind = OutgoingKind::Push(
        UserId::parse("@bob:push-retry.example").unwrap(),
        "retry-key".into(),
    );
    assert!(
        !queued_requests(&kind, 30).await.unwrap().is_empty(),
        "message must enqueue real push work"
    );
    guard.start();
    tokio::time::timeout(Duration::from_secs(5), async {
        while attempts.as_ref().load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Give the result branch a chance to run; failure must remain an active
    // durable row instead of being acknowledged as successful.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        active_requests_for(&kind)
            .await
            .unwrap()
            .iter()
            .any(|(_, value)| matches!(value,SendingEventType::Pdu(id) if id == &event))
    );
    tokio::time::timeout(Duration::from_secs(36), async {
        loop {
            if attempts.as_ref().load(Ordering::SeqCst) == 2
                && active_requests_for(&kind).await.unwrap().is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("gateway recovery must drain the same persistent request");
    assert_eq!(max_request().available_permits(), 1);
    peer.join().unwrap();
}

#[test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
fn http_push_retries_failed_gateway_without_acknowledging_queue() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(retry_push());
        })
        .unwrap()
        .join()
        .unwrap();
}
