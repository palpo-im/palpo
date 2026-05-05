use salvo::oapi::extract::JsonBody;
use salvo::prelude::*;
use url::Url;

use crate::core::client::push::{PusherAction, PushersResBody, SetPusherReqBody};
use crate::core::push::{Pusher, PusherKind};
use crate::data::DataError;
use crate::utils::url_guard;
use crate::{DepotExt, EmptyResult, JsonResult, data, empty_ok, hoops, json_ok};

pub fn authed_router() -> Router {
    Router::with_path("pushers")
        .get(pushers)
        .push(Router::with_hoop(hoops::limit_rate).push(Router::with_path("set").post(set_pusher)))
}

/// #GET /_matrix/client/r0/pushers
/// Gets all currently active pushers for the sender user.
#[endpoint]
async fn pushers(depot: &mut Depot) -> JsonResult<PushersResBody> {
    let authed = depot.authed_info()?;

    json_ok(PushersResBody {
        pushers: data::user::pusher::get_pushers(authed.user_id())?
            .into_iter()
            .map(TryInto::<Pusher>::try_into)
            .collect::<Result<Vec<_>, DataError>>()?,
    })
}

/// #POST /_matrix/client/r0/pushers/set
/// Adds a pusher for the sender user.
///
/// - TODO: Handle `append`
#[endpoint]
async fn set_pusher(body: JsonBody<SetPusherReqBody>, depot: &mut Depot) -> EmptyResult {
    let authed = depot.authed_info()?;
    let action = body.into_inner().0;
    if let PusherAction::Post(ref data) = action
        && let PusherKind::Http(ref http) = data.pusher.kind
    {
        // Validate the push-gateway URL up front so the user gets an
        // immediate, actionable error. The actual outbound request is also
        // gated by a CIDR-filtering DNS resolver in `push_gateway_client`.
        let url = Url::parse(&http.url).map_err(|e| {
            crate::core::MatrixError::invalid_param(format!("invalid pusher URL: {e}"))
        })?;
        url_guard::ensure_safe_outbound_url(&url)?;
    }
    crate::user::pusher::set_pusher(authed, action)?;
    empty_ok()
}
