use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::appservice::third_party::*;
use crate::core::third_party::Protocol;
use crate::{AuthArgs, JsonResult, MatrixError, json_ok};

pub fn router() -> Router {
    Router::with_path("thirdparty")
        .push(Router::with_path("protocol/{protocol}").get(get_protocol))
        .push(
            Router::with_path("location")
                .get(locations)
                .push(Router::with_path("{protocol}").get(protocol_locations)),
        )
        .push(
            Router::with_path("user")
                .get(users)
                .push(Router::with_path("{protocol}").get(protocol_users)),
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

/// # `GET /_matrix/app/v1/thirdparty/protocol/{protocol}`
///
/// Retrieve metadata about a specific third party protocol.
#[endpoint]
async fn get_protocol(aa: AuthArgs, protocol: PathParam<String>) -> JsonResult<ProtocolResBody> {
    verify_hs_token(&aa)?;

    let protocol_name = protocol.into_inner();

    // Look through registered appservices to find one that handles this protocol.
    let appservices = crate::appservices();
    for appservice in appservices {
        if let Some(protocols) = &appservice.protocols {
            if protocols.iter().any(|p| p == &protocol_name) {
                return json_ok(ProtocolResBody {
                    protocol: Protocol {
                        user_fields: vec![],
                        location_fields: vec![],
                        icon: String::new(),
                        field_types: Default::default(),
                        instances: vec![],
                    },
                });
            }
        }
    }

    Err(MatrixError::not_found("Protocol not found").into())
}

/// # `GET /_matrix/app/v1/thirdparty/location`
///
/// Retrieve third party locations from a Matrix room alias.
#[endpoint]
async fn locations(aa: AuthArgs) -> JsonResult<LocationsResBody> {
    verify_hs_token(&aa)?;
    json_ok(LocationsResBody::new(vec![]))
}

/// # `GET /_matrix/app/v1/thirdparty/location/{protocol}`
///
/// Retrieve Matrix portal rooms that lead to the matched third party location.
#[endpoint]
async fn protocol_locations(
    aa: AuthArgs,
    _args: ForProtocolReqArgs,
) -> JsonResult<LocationsResBody> {
    verify_hs_token(&aa)?;
    json_ok(LocationsResBody::new(vec![]))
}

/// # `GET /_matrix/app/v1/thirdparty/user`
///
/// Retrieve third party users from a Matrix User ID.
#[endpoint]
async fn users(aa: AuthArgs) -> JsonResult<UsersResBody> {
    verify_hs_token(&aa)?;
    json_ok(UsersResBody::new(vec![]))
}

/// # `GET /_matrix/app/v1/thirdparty/user/{protocol}`
///
/// Retrieve Matrix users bridged to the matched third party users.
#[endpoint]
async fn protocol_users(aa: AuthArgs, _args: ForProtocolReqArgs) -> JsonResult<UsersResBody> {
    verify_hs_token(&aa)?;
    json_ok(UsersResBody::new(vec![]))
}
