//! (De)serializable types for the [Matrix Server-Server API][federation-api].
//! These types are used by server code.
//!
//! [federation-api]: https://spec.matrix.org/latest/server-server-api/

mod backfill;
mod event;
pub(super) mod key;
mod media;
mod membership;
mod openid;
mod query;
mod room;
mod space;
mod threepid;
mod transaction;
mod user;

use salvo::prelude::*;

use crate::core::directory::Server;
use crate::core::federation::directory::ServerVersionResBody;
use crate::{AppError, AppResult, AuthArgs, JsonResult, config, hoops, json_ok};

pub fn router() -> Router {
    Router::with_path("federation")
        .hoop(check_federation_enabled)
        .oapi_tag("federation")
        // Discovery, OpenID tokens and identity-server bind callbacks do not use server
        // authentication.
        .push(Router::with_path("v1/version").get(version))
        .push(
            Router::with_path("v1")
                .push(threepid::router())
                .push(openid::router()),
        )
        // Historical non-standard v2 alias: the same explicit unsupported response.
        .push(
            Router::with_path("v2")
                .push(threepid::router())
                .push(openid::router()),
        )
        .push(authenticated_router())
}

fn authenticated_router() -> Router {
    let router = Router::new()
        .hoop(hoops::auth_by_access_token_or_signatures)
        .push(
            Router::with_path("v2")
                .push(backfill::router())
                .push(event::router())
                .push(membership::router_v2())
                .push(query::router())
                .push(room::router())
                .push(space::router())
                .push(transaction::router())
                .push(user::router()),
        )
        .push(
            Router::with_path("v1")
                .push(backfill::router())
                .push(event::router())
                .push(membership::router_v1())
                .push(query::router())
                .push(room::router())
                .push(space::router())
                .push(transaction::router())
                .push(user::router())
                .push(media::router()),
        )
        .push(Router::with_path("versions").get(get_versions));

    #[cfg(feature = "unstable-msc4495")]
    let router = router.push(Router::with_path("unstable").push(query::unstable_router()));

    router
}

#[handler]
async fn check_federation_enabled() -> AppResult<()> {
    let conf = config::get();
    if conf.enabled_federation().is_none() {
        Err(AppError::public("Federation is disabled."))
    } else {
        Ok(())
    }
}

#[endpoint]
async fn get_versions(_aa: AuthArgs) -> JsonResult<serde_json::Value> {
    json_ok(serde_json::json!({
        "versions": ["v1"]
    }))
}
/// #GET /_matrix/federation/v1/version
/// Get version information on this server.
#[endpoint]
async fn version() -> JsonResult<ServerVersionResBody> {
    json_ok(ServerVersionResBody {
        server: Some(Server {
            name: Some("Palpo".to_owned()),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        }),
    })
}

#[cfg(test)]
mod tests {
    use salvo::http::{Method, Request, StatusCode};
    use salvo::prelude::{Router, Service};
    use salvo::routing::PathState;
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::{config, router};

    fn service() -> Service {
        config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name": "federation.example",
                "db": {"url": "unused-test-config"},
                "federation": {"enable": true},
            }))
            .unwrap()
        });
        Service::new(Router::with_path("_matrix").push(router()))
    }

    #[tokio::test]
    async fn version_is_public_and_returns_server_information() {
        let service = service();
        for authorization in [None, Some("Bearer invalid-token")] {
            let request = TestClient::get("http://localhost/_matrix/federation/v1/version");
            let request = match authorization {
                Some(value) => request.add_header("authorization", value, true),
                None => request,
            };
            let mut response = request.send(&service).await;
            assert_eq!(response.status_code, Some(StatusCode::OK));
            let content_type = response
                .headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .parse::<mime::Mime>()
                .unwrap();
            assert_eq!(content_type.essence_str(), "application/json");
            assert_eq!(
                response.take_json::<Value>().await.unwrap(),
                json!({"server": {"name": "Palpo", "version": env!("CARGO_PKG_VERSION")}})
            );
        }
    }

    #[tokio::test]
    async fn version_does_not_accept_post() {
        let mut request = Request::default();
        *request.method_mut() = Method::POST;
        let mut path = PathState::from_owned_path("/federation/v1/version".to_owned());
        assert!(router().detect(&mut request, &mut path).await.is_none());
    }

    #[tokio::test]
    async fn other_federation_routes_still_require_authentication() {
        let service = service();
        for version in ["v1", "v2"] {
            let mut response = TestClient::get(format!(
                "http://localhost/_matrix/federation/{version}/event/$test"
            ))
            .send(&service)
            .await;
            assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
            assert_eq!(
                response.take_json::<Value>().await.unwrap()["errcode"],
                "M_MISSING_TOKEN"
            );
        }
    }

    #[tokio::test]
    #[ignore = "run alone; owns process-global configuration"]
    async fn review_onbind_respects_disabled_federation() {
        config::CONFIG
            .set(
                serde_json::from_value(json!({
                    "server_name":"onbind-disabled.example","db":{"url":"unused"},
                    "federation":{"enable":false}
                }))
                .unwrap(),
            )
            .unwrap();
        let service = Service::new(Router::with_path("_matrix").push(router()));
        for version in ["v1", "v2"] {
            for method in [Method::POST, Method::PUT] {
                let url = format!("http://localhost/_matrix/federation/{version}/3pid/onbind");
                let request = if method == Method::POST {
                    TestClient::post(url)
                } else {
                    TestClient::put(url)
                };
                let mut response = request.json(&json!({})).send(&service).await;
                assert_ne!(response.status_code, Some(StatusCode::OK));
                let body = response.take_json::<Value>().await.unwrap();
                assert_eq!(body["error"], "Federation is disabled.");
                assert_ne!(body["errcode"], "M_MISSING_TOKEN");
            }
        }
    }
}
