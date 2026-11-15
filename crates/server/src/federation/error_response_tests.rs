//! Real HTTP error bodies must be bounded before the media layer sees them.
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

async fn serve(wire: Vec<u8>, signed: bool) -> (url::Url, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        if signed {
            assert!(
                String::from_utf8_lossy(&request)
                    .to_ascii_lowercase()
                    .contains("authorization: x-matrix ")
            );
        }
        if socket.write_all(&wire).await.is_ok() {
            // An attacker need not finish the body. A bounded reader must reject
            // it without waiting for EOF or the ordinary federation timeout.
            assert!(!matches!(socket.read(&mut byte).await, Ok(1..)));
        }
    });
    (
        url::Url::parse(&format!("http://{address}/media")).unwrap(),
        task,
    )
}

async fn peer(wire: Vec<u8>) -> (reqwest::Response, tokio::task::JoinHandle<()>) {
    let (url, task) = serve(wire, false).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .unwrap();
    (response, task)
}

async fn assert_closed(mut task: tokio::task::JoinHandle<()>) {
    match tokio::time::timeout(Duration::from_secs(3), &mut task).await {
        Ok(result) => result.unwrap(),
        Err(_) => {
            task.abort();
            panic!("dropping an oversized error response must close the incomplete HTTP body");
        }
    }
}

#[tokio::test]
#[ignore = "owns process-global federation configuration; run alone"]
async fn review_federation_error_transport_is_bounded() {
    config::CONFIG
        .set(
            serde_json::from_value(serde_json::json!({
                "server_name":"local-error-budget.example", "db":{"url":"unused"},
                "federation":{"enable":true}, "ip_range_denylist":[],
                "http_client":{"federation_retries":0}
            }))
            .unwrap(),
        )
        .unwrap();
    let destination = ServerName::parse("error-budget.example").unwrap();
    let fixed = format!(
        "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\n\r\n",
        MAX_ERROR_RESPONSE_BYTES + 1
    )
    .into_bytes();
    let mut chunked = format!(
        "HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
        MAX_ERROR_RESPONSE_BYTES + 1
    )
    .into_bytes();
    chunked.extend(std::iter::repeat_n(b'x', MAX_ERROR_RESPONSE_BYTES + 1));
    chunked.extend_from_slice(b"\r\n");
    for wire in [fixed, chunked] {
        let (url, task) = serve(wire, true).await;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            send_request(
                &destination,
                reqwest::Request::new(reqwest::Method::GET, url),
            ),
        )
        .await
        .expect("the actual signed federation transport must enforce the error budget");
        let Err(AppError::Matrix(error)) = result else {
            panic!("expected a bounded Matrix error")
        };
        assert_eq!(error.status_code, Some(reqwest::StatusCode::NOT_FOUND));
        assert!(serde_json::to_vec(&error.body).unwrap().len() < MAX_ERROR_LOG_BYTES);
        assert_closed(task).await;
    }
    let (url, task) = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec(),
        true,
    )
    .await;
    let response = send_request(
        &destination,
        reqwest::Request::new(reqwest::Method::GET, url),
    )
    .await
    .unwrap();
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"ok");
    task.abort();
}

async fn parse(response: reqwest::Response) -> MatrixError {
    let destination = ServerName::parse("error-budget.example").unwrap();
    let url = response.url().clone();
    tokio::time::timeout(
        Duration::from_secs(3),
        error_response(&destination, &url, response),
    )
    .await
    .expect("error response must stop at its byte budget without waiting for EOF")
}

#[tokio::test]
async fn review_federation_error_rejects_oversized_length_before_reading_body() {
    let (response, task) = peer(
        format!(
            "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\n\r\n",
            MAX_ERROR_RESPONSE_BYTES + 1
        )
        .into_bytes(),
    )
    .await;
    let error = parse(response).await;
    assert_eq!(error.status_code, Some(reqwest::StatusCode::NOT_FOUND));
    assert!(matches!(error.kind, ErrorKind::Unknown));
    assert!(serde_json::to_vec(&error.body).unwrap().len() < MAX_ERROR_LOG_BYTES);
    assert_closed(task).await;
}

#[tokio::test]
async fn review_federation_error_rejects_oversized_chunked_body_without_eof() {
    let mut wire = b"HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    wire.extend_from_slice(format!("{:x}\r\n", MAX_ERROR_RESPONSE_BYTES + 1).as_bytes());
    wire.extend(std::iter::repeat_n(b'x', MAX_ERROR_RESPONSE_BYTES + 1));
    wire.extend_from_slice(b"\r\n");
    let (response, task) = peer(wire).await;
    let error = parse(response).await;
    assert_eq!(error.status_code, Some(reqwest::StatusCode::NOT_FOUND));
    assert!(matches!(error.kind, ErrorKind::Unknown));
    assert!(serde_json::to_vec(&error.body).unwrap().len() < MAX_ERROR_LOG_BYTES);
    assert_closed(task).await;
}

#[tokio::test]
#[tracing_test::traced_test]
async fn review_federation_error_preserves_matrix_errors_and_truncates_utf8_logs() {
    let message = format!("{}UNLOGGED_ERROR_TAIL_20261115", "界".repeat(2000));
    let body = serde_json::to_vec(&serde_json::json!({
        "errcode":"M_FORBIDDEN", "error":message
    }))
    .unwrap();
    let mut wire = format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    wire.extend_from_slice(&body);
    let (response, task) = peer(wire).await;
    let error = parse(response).await;
    assert_eq!(error.status_code, Some(reqwest::StatusCode::FORBIDDEN));
    assert!(matches!(error.kind, ErrorKind::Forbidden));
    assert_eq!(serde_json::to_value(&error.body).unwrap()["error"], message);
    assert!(logs_contain("truncated=true"));
    assert!(!logs_contain("UNLOGGED_ERROR_TAIL_20261115"));
    task.abort();
}

#[tokio::test]
async fn review_federation_error_preserves_authentication_and_retry_metadata() {
    for (status, headers, body) in [
        (
            "401 Unauthorized",
            "WWW-Authenticate: Bearer error=\"insufficient_scope\", scope=\"matrix:read\"\r\n",
            serde_json::json!({"errcode":"M_FORBIDDEN", "error":"scope required"}),
        ),
        (
            "429 Too Many Requests",
            "",
            serde_json::json!({"errcode":"M_LIMIT_EXCEEDED", "error":"retry later", "retry_after_ms":1234}),
        ),
    ] {
        let bytes = serde_json::to_vec(&body).unwrap();
        let mut wire = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\n\r\n",
            bytes.len()
        )
        .into_bytes();
        wire.extend_from_slice(&bytes);
        let (response, task) = peer(wire).await;
        let error = parse(response).await;
        if status.starts_with("401") {
            assert_eq!(error.status_code, Some(reqwest::StatusCode::UNAUTHORIZED));
            assert_eq!(
                error.authenticate,
                Some(AuthenticateError::InsufficientScope {
                    scope: "matrix:read".into()
                })
            );
            assert!(matches!(error.kind, ErrorKind::Forbidden));
        } else {
            assert_eq!(
                error.status_code,
                Some(reqwest::StatusCode::TOO_MANY_REQUESTS)
            );
            assert_eq!(
                serde_json::to_value(&error.kind).unwrap()["retry_after_ms"],
                1234
            );
        }
        assert_eq!(
            serde_json::to_value(&error.body).unwrap()["error"],
            body["error"]
        );
        task.abort();
    }
}
