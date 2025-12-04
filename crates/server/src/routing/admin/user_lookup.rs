//! User lookup endpoints for MAS integration
//!
//! - GET /_synapse/admin/v1/auth_providers/{provider}/users/{external_id}
//! - GET /_synapse/admin/v1/threepid/{medium}/users/{address}

use salvo::oapi::extract::PathParam;
use salvo::prelude::*;
use serde::Serialize;

use crate::routing::prelude::*;

/// Response for user lookup endpoints
#[derive(Debug, Serialize, ToSchema)]
pub struct UserIdResponse {
    pub user_id: String,
}

/// GET /_synapse/admin/v1/auth_providers/{provider}/users/{external_id}
///
/// Find a user based on an external ID from an auth provider (SSO/OIDC)
#[endpoint]
pub async fn get_user_by_external_id(
    provider: PathParam<String>,
    external_id: PathParam<String>,
) -> JsonResult<UserIdResponse> {
    let provider = provider.into_inner();
    let external_id = external_id.into_inner();

    let user_id = crate::data::user::get_user_by_external_id(&provider, &external_id)?
        .ok_or_else(|| MatrixError::not_found("User not found"))?;

    json_ok(UserIdResponse {
        user_id: user_id.to_string(),
    })
}

/// GET /_synapse/admin/v1/threepid/{medium}/users/{address}
///
/// Find a user based on 3PID (email, phone, etc.)
#[endpoint]
pub async fn get_user_by_threepid(
    medium: PathParam<String>,
    address: PathParam<String>,
) -> JsonResult<UserIdResponse> {
    let medium = medium.into_inner();
    let address = address.into_inner();

    let user_id = crate::data::user::get_user_by_threepid(&medium, &address)?
        .ok_or_else(|| MatrixError::not_found("User not found"))?;

    json_ok(UserIdResponse {
        user_id: user_id.to_string(),
    })
}

pub fn router() -> Router {
    Router::new()
        .push(
            Router::with_path("v1/auth_providers/<provider>/users/<external_id>")
                .get(get_user_by_external_id),
        )
        .push(
            Router::with_path("v1/threepid/<medium>/users/<address>")
                .get(get_user_by_threepid),
        )
}
