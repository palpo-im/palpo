use std::time::Duration;

use salvo::oapi::extract::{JsonBody, *};
use salvo::prelude::*;

use crate::core::OwnedUserId;
use crate::core::client::presence::{PresenceResBody, SetPresenceReqBody};
use crate::room::state;
use crate::{
    AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, config, empty_ok, hoops, json_ok,
};

pub fn authed_router() -> Router {
    Router::with_path("presence/{user_id}/status")
        .get(get_status)
        .push(Router::with_hoop(hoops::limit_rate).put(set_status))
}

/// #GET /_matrix/client/r0/presence/{user_id}/status
/// Gets the presence state of the given user.
///
/// - Only works if you share a room with the user
#[endpoint]
fn get_status(user_id: PathParam<OwnedUserId>, depot: &mut Depot) -> JsonResult<PresenceResBody> {
    if !crate::config::get().presence.allow_local {
        return Err(MatrixError::forbidden("presence is disabled on this server", None).into());
    }

    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let user_id = user_id.into_inner();

    if !state::user_can_see_user(sender_id, &user_id)? {
        return Err(
            MatrixError::unauthorized("you cannot get the presence state of this user").into(),
        );
    }

    let content = crate::data::user::last_presence(&user_id)?.content;

    json_ok(PresenceResBody {
        // TODO: Should just use the presenceeventcontent type here?
        status_msg: content.status_msg,
        currently_active: content.currently_active,
        last_active_ago: content.last_active_ago.map(Duration::from_millis),
        presence: content.presence,
    })
}

/// #PUT /_matrix/client/r0/presence/{user_id}/status
/// Sets the presence state of the sender user.
#[endpoint]
async fn set_status(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    body: JsonBody<SetPresenceReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    if !config::get().presence.allow_local {
        return Err(MatrixError::forbidden("presence is disabled on this server", None).into());
    }

    let authed = depot.authed_info()?;
    let user_id = user_id.into_inner();
    if authed.user_id() != user_id {
        if let Some(appservice) = authed.appservice() {
            if !appservice.is_user_match(&user_id) {
                return Err(MatrixError::forbidden(
                    "appservice cannot set the presence state of user not matched by its namespace",
                    None,
                )
                .into());
            }
        } else {
            return Err(MatrixError::forbidden(
                "you cannot set the presence state of another user",
                None,
            )
            .into());
        }
    }
    let SetPresenceReqBody {
        presence,
        status_msg,
    } = body.into_inner();
    crate::user::set_presence(authed.user_id(), Some(presence), status_msg.clone(), true)?;

    empty_ok()
}
