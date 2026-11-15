//! A real remote TLS media peer must never set browser policies on our origin.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use salvo::conn::Acceptor;
use salvo::conn::rustls::{Keycert, RustlsConfig};
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::json;

use crate::core::{OwnedDeviceId, UnixMillis, UserId};
use crate::server_key::test_fixture as tls_fixture;
use crate::{config, data, storage};

const IMAGE: &[u8] = b"\x89PNG\r\n\x1a\ncontrolled-thumbnail";
const HTML: &[u8] = b"<script>parent.thumbnailExecuted=true</script>";

#[derive(Clone)]
struct Peer(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Handler for Peer {
    async fn handle(
        &self,
        req: &mut Request,
        _depot: &mut Depot,
        res: &mut Response,
        _ctrl: &mut FlowCtrl,
    ) {
        assert!(req.uri().path().starts_with("/_matrix/media/v3/thumbnail/"));
        assert!(
            req.headers()["authorization"]
                .to_str()
                .unwrap()
                .starts_with("X-Matrix ")
        );
        self.0.fetch_add(1, Ordering::SeqCst);
        let html = req.uri().path().ends_with("/html");
        res.add_header(
            "Content-Type",
            if html { "text/html" } else { "image/png" },
            true,
        )
        .unwrap();
        res.add_header(
            "Content-Disposition",
            "inline; filename=untrusted.html",
            true,
        )
        .unwrap();
        res.add_header(
            "Set-Cookie",
            "oidc_session=attacker; Path=/; Secure; HttpOnly; SameSite=Lax",
            true,
        )
        .unwrap();
        res.add_header("Refresh", "0; url=https://evil.example/", true)
            .unwrap();
        res.add_header("Access-Control-Allow-Origin", "https://evil.example", true)
            .unwrap();
        res.add_header("Access-Control-Allow-Credentials", "true", true)
            .unwrap();
        res.add_header(
            "Content-Security-Policy",
            "default-src * 'unsafe-inline'",
            true,
        )
        .unwrap();
        res.add_header("Cross-Origin-Resource-Policy", "same-origin", true)
            .unwrap();
        res.add_header("X-Content-Type-Options", "wrong", true)
            .unwrap();
        res.write_body(if html { HTML } else { IMAGE }).unwrap();
    }
}

async fn send(request: RequestBuilder, service: &Service) -> Response {
    let router = service.router();
    tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap()
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn http_remote_thumbnail_strips_untrusted_response_headers() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn run() {
    crate::test_database::init();
    let dir = tempfile::tempdir().unwrap();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"thumbnail-headers.example", "db":{"url":"unused"},
                "ip_range_denylist":[], "allow_invalid_tls_certificates":true,
                "federation":{"enable":true},
                "storage":{"backend":"fs", "root":dir.path().to_str().unwrap()},
            }))
            .unwrap(),
        )
        .unwrap();
    storage::init(&config::get().storage).unwrap();
    let user = UserId::parse("@viewer:thumbnail-headers.example").unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        localpart: "viewer".into(),
        server_name: user.server_name().to_owned(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::device::create_device(
        &user,
        &OwnedDeviceId::from("D"),
        "thumbnail-viewer-token",
        None,
        None,
    )
    .await
    .unwrap();

    let _ = rustls::crypto::ring::default_provider().install_default();
    let acceptor = TcpListener::new("127.0.0.1:0")
        .rustls(RustlsConfig::new(
            Keycert::new().cert(tls_fixture::CERT).key(tls_fixture::KEY),
        ))
        .bind()
        .await;
    let address = acceptor.holdings()[0]
        .local_addr
        .clone()
        .into_std()
        .unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let peer = tokio::spawn(
        Server::new(acceptor).serve(Router::with_path("{*path}").get(Peer(requests.clone()))),
    );
    let service = Service::new(crate::routing::root());

    for (prefix, media, authenticated) in [
        ("/_matrix/media/v3", "png", false),
        ("/_matrix/client/v1/media", "png-authed", true),
        ("/_matrix/media/v3", "html", false),
    ] {
        let request = TestClient::get(format!(
            "http://localhost{prefix}/thumbnail/127.0.0.1:{}/{media}?width=32&height=32",
            address.port()
        ));
        let request = if authenticated {
            request.add_header("Authorization", "Bearer thumbnail-viewer-token", true)
        } else {
            request
        };
        let mut response = send(request, &service).await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        for header in [
            "set-cookie",
            "refresh",
            "access-control-allow-origin",
            "access-control-allow-credentials",
            "content-security-policy",
        ] {
            assert!(
                !response.headers().contains_key(header),
                "forwarded {header}"
            );
        }
        assert_eq!(response.headers()["X-Content-Type-Options"], "nosniff");
        assert_eq!(
            response.headers()["Cross-Origin-Resource-Policy"],
            "cross-origin"
        );
        assert_eq!(
            response.headers()["Content-Type"],
            if media == "html" {
                "text/html"
            } else {
                "image/png"
            }
        );
        assert_eq!(
            response.headers()["Content-Disposition"],
            if media == "html" {
                "attachment"
            } else {
                "inline"
            }
        );
        let body = response.take_bytes(None).await.unwrap();
        assert_eq!(body.as_ref(), if media == "html" { HTML } else { IMAGE });
    }
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    peer.abort();
}
