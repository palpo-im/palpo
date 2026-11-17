//! Matrix IDs use percent-encoded separators as data in a single URL segment.

use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};

#[handler]
fn profile(req: &mut Request) -> String {
    format!(
        "profile|{}|{}",
        req.param::<String>("user_id").expect("user ID"),
        req.uri().path_and_query().expect("request target").as_str()
    )
}

#[handler]
fn display_name(req: &mut Request) -> String {
    format!(
        "displayname|{}",
        req.param::<String>("user_id").expect("user ID")
    )
}

#[handler]
fn make_join(req: &mut Request) -> String {
    format!(
        "{}|{}|{}",
        req.param::<String>("room_id").expect("room ID"),
        req.param::<String>("user_id").expect("user ID"),
        req.uri().path_and_query().expect("request target").as_str()
    )
}

fn service() -> Service {
    // The direct core dependency enforces the routing version, rather than
    // relying on the facade's caret requirements for its internal crates.
    let router: salvo_core::Router = Router::new()
        .push(
            Router::with_path("_matrix/client/v3/profile/{user_id}")
                .get(profile)
                .push(Router::with_path("displayname").get(display_name)),
        )
        .push(
            Router::with_path("_matrix/federation/v1/make_join/{room_id}/{user_id}").get(make_join),
        );
    Service::new(router)
}

#[tokio::test]
async fn client_ids_decode_once_without_changing_route_segments_or_signed_uri() {
    let service = service();
    for (encoded, expected) in [
        ("@alice:example.org", "@alice:example.org"),
        ("%40alice%3Aexample.org", "@alice:example.org"),
        ("%40a%2Fb%3Aexample.org", "@a/b:example.org"),
        ("@a%2fb:example.org", "@a/b:example.org"),
        ("%40a%252Fb%3aexample.org", "@a%2Fb:example.org"),
        ("@a%2Fdisplayname:example.org", "@a/displayname:example.org"),
    ] {
        let path = format!("/_matrix/client/v3/profile/{encoded}?probe=%2F&literal=%252F");
        let mut response = TestClient::get(format!("http://localhost{path}"))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK), "{path}");
        assert_eq!(
            response.take_string().await.expect("profile response"),
            format!("profile|{expected}|{path}")
        );
    }

    let mut response = TestClient::get(
        "http://localhost/_matrix/client/v3/profile/@a%2Fb:example.org/displayname",
    )
    .send(&service)
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(
        response.take_string().await.expect("display name response"),
        "displayname|@a/b:example.org"
    );
}

#[tokio::test]
async fn federation_identifiers_preserve_encoded_slashes_and_original_uri() {
    let service = service();
    let path = "/_matrix/federation/v1/make_join/%21room%2fpart%3Aexample.org/%40a%2Fb:example.org?ver=12&via=example.org";
    let mut response = TestClient::get(format!("http://localhost{path}"))
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    assert_eq!(
        response.take_string().await.expect("make join response"),
        format!("!room/part:example.org|@a/b:example.org|{path}")
    );
}

#[tokio::test]
async fn literal_and_encoded_slashes_do_not_select_the_same_route() {
    let service = service();
    for path in [
        "/_matrix/client/v3/profile/@a/b:example.org",
        "/_matrix%2Fclient/v3/profile/@a:example.org",
    ] {
        let response = TestClient::get(format!("http://localhost{path}"))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND), "{path}");
    }
}
