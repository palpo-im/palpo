use std::collections::BTreeMap;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::third_party::*;
use crate::{AuthArgs, JsonResult, json_ok, sending};

pub fn authed_router() -> Router {
    Router::with_path("thirdparty")
        .push(Router::with_path("protocols").get(protocols))
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

/// Build a GET request to an appservice's thirdparty endpoint.
fn appservice_get_request(base_url: &str, path: &str) -> Result<reqwest::Request, crate::AppError> {
    let url = reqwest::Url::parse(&format!("{base_url}/_matrix/app/v1/thirdparty/{path}"))
        .map_err(|e| crate::AppError::public(format!("invalid appservice URL: {e}")))?;
    Ok(reqwest::Request::new(reqwest::Method::GET, url))
}

/// Percent-encode a string for use in a URL query parameter.
fn encode_query_value(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// # `GET /_matrix/client/v3/thirdparty/protocols`
///
/// Fetches metadata about all third party protocols supported by the homeserver.
/// Forwards requests to all registered appservices that declare protocols.
#[endpoint]
async fn protocols(_aa: AuthArgs) -> JsonResult<ProtocolsResBody> {
    let mut result = BTreeMap::new();

    for appservice in crate::appservice::all()?.values() {
        let Some(proto_list) = &appservice.registration.protocols else {
            continue;
        };
        let Some(base_url) = &appservice.registration.url else {
            continue;
        };

        for protocol_id in proto_list {
            let path = format!("protocol/{protocol_id}");
            let Ok(request) = appservice_get_request(base_url, &path) else {
                continue;
            };
            match sending::send_appservice_request::<Protocol>(
                appservice.registration.clone(),
                request,
            )
            .await
            {
                Ok(proto) => {
                    result.insert(protocol_id.clone(), proto);
                }
                Err(e) => {
                    warn!(
                        "Failed to fetch protocol '{}' from appservice '{}': {}",
                        protocol_id, appservice.registration.id, e
                    );
                }
            }
        }
    }

    json_ok(ProtocolsResBody::new(result))
}

/// # `GET /_matrix/client/v3/thirdparty/protocol/{protocol}`
///
/// Fetches metadata about a specific third party protocol.
#[endpoint]
async fn get_protocol(_aa: AuthArgs, protocol: PathParam<String>) -> JsonResult<ProtocolResBody> {
    let protocol_id = protocol.into_inner();

    for appservice in crate::appservice::all()?.values() {
        let Some(proto_list) = &appservice.registration.protocols else {
            continue;
        };
        if !proto_list.iter().any(|p| p == &protocol_id) {
            continue;
        }
        let Some(base_url) = &appservice.registration.url else {
            continue;
        };

        let path = format!("protocol/{protocol_id}");
        let request = appservice_get_request(base_url, &path)?;
        let protocol_data = sending::send_appservice_request::<Protocol>(
            appservice.registration.clone(),
            request,
        )
        .await?;

        return json_ok(ProtocolResBody::new(protocol_data));
    }

    Err(crate::MatrixError::not_found("Protocol not found").into())
}

/// # `GET /_matrix/client/v3/thirdparty/location`
///
/// Retrieve third party locations from a Matrix room alias.
#[endpoint]
async fn locations(_aa: AuthArgs, req: &mut Request) -> JsonResult<LocationsResBody> {
    let alias = req.query::<String>("alias");
    let mut result = Vec::new();

    for appservice in crate::appservice::all()?.values() {
        if appservice.registration.protocols.is_none() {
            continue;
        }
        let Some(base_url) = &appservice.registration.url else {
            continue;
        };

        let path = match &alias {
            Some(a) => format!("location?alias={}", encode_query_value(a)),
            None => "location".to_owned(),
        };
        let Ok(request) = appservice_get_request(base_url, &path) else {
            continue;
        };
        match sending::send_appservice_request::<Vec<Location>>(
            appservice.registration.clone(),
            request,
        )
        .await
        {
            Ok(locs) => result.extend(locs),
            Err(e) => {
                debug!(
                    "Failed to fetch locations from appservice '{}': {}",
                    appservice.registration.id, e
                );
            }
        }
    }

    json_ok(LocationsResBody::new(result))
}

/// # `GET /_matrix/client/v3/thirdparty/location/{protocol}`
///
/// Retrieve Matrix portal rooms that lead to the matched third party location.
#[endpoint]
async fn protocol_locations(
    _aa: AuthArgs,
    protocol: PathParam<String>,
) -> JsonResult<LocationsResBody> {
    let protocol_id = protocol.into_inner();
    let mut result = Vec::new();

    for appservice in crate::appservice::all()?.values() {
        let Some(proto_list) = &appservice.registration.protocols else {
            continue;
        };
        if !proto_list.iter().any(|p| p == &protocol_id) {
            continue;
        }
        let Some(base_url) = &appservice.registration.url else {
            continue;
        };

        let path = format!("location/{protocol_id}");
        let Ok(request) = appservice_get_request(base_url, &path) else {
            continue;
        };
        match sending::send_appservice_request::<Vec<Location>>(
            appservice.registration.clone(),
            request,
        )
        .await
        {
            Ok(locs) => result.extend(locs),
            Err(e) => {
                debug!(
                    "Failed to fetch locations for '{}' from appservice '{}': {}",
                    protocol_id, appservice.registration.id, e
                );
            }
        }
    }

    json_ok(LocationsResBody::new(result))
}

/// # `GET /_matrix/client/v3/thirdparty/user`
///
/// Retrieve third party users from a Matrix User ID.
#[endpoint]
async fn users(_aa: AuthArgs, req: &mut Request) -> JsonResult<UsersResBody> {
    let userid = req.query::<String>("userid");
    let mut result = Vec::new();

    for appservice in crate::appservice::all()?.values() {
        if appservice.registration.protocols.is_none() {
            continue;
        }
        let Some(base_url) = &appservice.registration.url else {
            continue;
        };

        let path = match &userid {
            Some(u) => format!("user?userid={}", encode_query_value(u)),
            None => "user".to_owned(),
        };
        let Ok(request) = appservice_get_request(base_url, &path) else {
            continue;
        };
        match sending::send_appservice_request::<Vec<User>>(
            appservice.registration.clone(),
            request,
        )
        .await
        {
            Ok(user_list) => result.extend(user_list),
            Err(e) => {
                debug!(
                    "Failed to fetch users from appservice '{}': {}",
                    appservice.registration.id, e
                );
            }
        }
    }

    json_ok(UsersResBody::new(result))
}

/// # `GET /_matrix/client/v3/thirdparty/user/{protocol}`
///
/// Retrieve Matrix users bridged to the matched third party users.
#[endpoint]
async fn protocol_users(
    _aa: AuthArgs,
    protocol: PathParam<String>,
) -> JsonResult<UsersResBody> {
    let protocol_id = protocol.into_inner();
    let mut result = Vec::new();

    for appservice in crate::appservice::all()?.values() {
        let Some(proto_list) = &appservice.registration.protocols else {
            continue;
        };
        if !proto_list.iter().any(|p| p == &protocol_id) {
            continue;
        }
        let Some(base_url) = &appservice.registration.url else {
            continue;
        };

        let path = format!("user/{protocol_id}");
        let Ok(request) = appservice_get_request(base_url, &path) else {
            continue;
        };
        match sending::send_appservice_request::<Vec<User>>(
            appservice.registration.clone(),
            request,
        )
        .await
        {
            Ok(user_list) => result.extend(user_list),
            Err(e) => {
                debug!(
                    "Failed to fetch users for '{}' from appservice '{}': {}",
                    protocol_id, appservice.registration.id, e
                );
            }
        }
    }

    json_ok(UsersResBody::new(result))
}
