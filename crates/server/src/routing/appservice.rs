//! Endpoints for the [Matrix Application Service API][appservice-api].
//!
//! [appservice-api]: https://spec.matrix.org/latest/application-service-api/
mod third_party;
mod transaction;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::appservice::ping::SendPingReqBody;
use crate::{AuthArgs, EmptyResult, MatrixError, empty_ok};

pub fn router() -> Router {
    Router::with_path("app").oapi_tag("appservice").push(
        Router::with_path("v1")
            .push(Router::with_path("ping").post(ping))
            .push(Router::with_path("rooms/{room_alias}").get(query_rooms))
            .push(Router::with_path("users/{user_id}").get(query_users))
            .push(third_party::router())
            .push(transaction::router()),
    )
}

fn verify_hs_token(aa: &AuthArgs) -> Result<(), crate::AppError> {
    let token = aa.require_access_token()?;
    let appservices = crate::appservices();
    if appservices.iter().any(|a| a.hs_token == token) {
        Ok(())
    } else {
        Err(MatrixError::forbidden("Invalid hs_token", None).into())
    }
}

/// # `POST /_matrix/app/v1/ping`
///
/// Ping the application service to check it is alive.
#[endpoint]
async fn ping(aa: AuthArgs, body: JsonBody<SendPingReqBody>) -> EmptyResult {
    verify_hs_token(&aa)?;
    let body = body.into_inner();
    debug!(
        "Appservice ping received, transaction_id: {:?}",
        body.transaction_id
    );
    empty_ok()
}

/// # `GET /_matrix/app/v1/rooms/{roomAlias}`
///
/// Query the existence of a room alias on the application service.
/// Returns 200 if the alias is known, 404 otherwise.
#[endpoint]
async fn query_rooms(aa: AuthArgs, room_alias: PathParam<String>) -> EmptyResult {
    verify_hs_token(&aa)?;

    let room_alias = room_alias.into_inner();

    // Check if any registered appservice has a namespace that matches this alias.
    let appservices = crate::appservices();
    for appservice in appservices {
        for ns in &appservice.namespaces.aliases {
            if let Ok(re) = regex::Regex::new(&ns.regex) {
                if re.is_match(&room_alias) {
                    debug!(
                        "Appservice '{}' claims room alias {}",
                        appservice.id, room_alias
                    );
                    return empty_ok();
                }
            }
        }
    }

    Err(MatrixError::not_found("Room alias not found").into())
}

/// # `GET /_matrix/app/v1/users/{userId}`
///
/// Query the existence of a user ID on the application service.
/// Returns 200 if the user is known, 404 otherwise.
#[endpoint]
async fn query_users(aa: AuthArgs, user_id: PathParam<String>) -> EmptyResult {
    verify_hs_token(&aa)?;

    let user_id = user_id.into_inner();

    // Check if any registered appservice has a namespace that matches this user ID.
    let appservices = crate::appservices();
    for appservice in appservices {
        for ns in &appservice.namespaces.users {
            if let Ok(re) = regex::Regex::new(&ns.regex) {
                if re.is_match(&user_id) {
                    debug!("Appservice '{}' claims user {}", appservice.id, user_id);
                    return empty_ok();
                }
            }
        }
    }

    Err(MatrixError::not_found("User not found").into())
}
