use std::time::Duration;

use salvo::oapi::extract::{JsonBody, *};
use salvo::prelude::*;

use crate::core::client::presence::{PresenceResBody, SetPresenceReqBody};
use crate::core::{OwnedUserId, UserId};
#[cfg(not(feature = "unstable-msc4495"))]
use crate::room::state;
use crate::{
    AppResult, AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, config, empty_ok, hoops,
    json_ok,
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
async fn get_status(
    user_id: PathParam<OwnedUserId>,
    depot: &mut Depot,
    req: &mut Request,
) -> JsonResult<serde_json::Value> {
    if !crate::config::get().presence.allow_local {
        return Err(MatrixError::forbidden("presence is disabled on this server", None).into());
    }

    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let user_id = user_id.into_inner();

    if !presence_visible_to(&user_id, sender_id).await? {
        return Err(
            MatrixError::unauthorized("you cannot get the presence state of this user").into(),
        );
    }

    let content = crate::data::user::last_presence(&user_id).await?.content;

    let body = PresenceResBody {
        // TODO: Should just use the presenceeventcontent type here?
        status_msg: content.status_msg,
        currently_active: content.currently_active,
        last_active_ago: content.last_active_ago.map(Duration::from_millis),
        presence: content.presence,
    };
    #[allow(unused_mut)]
    let mut value = serde_json::to_value(body)?;
    #[cfg(feature = "unstable-msc4532")]
    if let Some(status) = content.status {
        value[crate::core::presence::revised::STATUS] = status;
    }
    #[cfg(feature = "unstable-msc4532")]
    crate::user::presence::revised::project(
        &user_id,
        &mut value,
        req.query::<bool>(crate::core::presence::revised::REVISED)
            .unwrap_or(false),
    )
    .await?;
    #[cfg(not(feature = "unstable-msc4532"))]
    let _ = req;
    json_ok(value)
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
    #[cfg(feature = "unstable-msc4532")]
    {
        crate::core::presence::revised::State::parse(presence.as_str(), true)
            .ok_or_else(|| MatrixError::invalid_param("Unknown presence state"))?;
        if let Some(message) = status_msg {
            crate::user::presence::revised::legacy_message(&user_id, message).await?;
        }
        let client = if let Some(service) = authed.appservice() {
            format!("as:{}", service.registration.id)
        } else {
            format!("device:{}", authed.require_device_id()?)
        };
        crate::user::presence::revised::touch(
            &user_id,
            &client,
            presence.as_str(),
            authed
                .appservice()
                .is_some_and(|service| service.is_exclusive_user_match(&user_id)),
        )
        .await?;
    }
    #[cfg(not(feature = "unstable-msc4532"))]
    crate::user::set_presence(authed.user_id(), Some(presence), status_msg.clone(), true).await?;

    empty_ok()
}

/// Whether `viewer_id` may be shown `sender_id`'s presence.
///
/// The same rule `/sync` applies. Authorizing a direct read on shared rooms alone would let
/// any of them poll for presence the sender never agreed to share (MSC4495).
async fn presence_visible_to(sender_id: &UserId, viewer_id: &UserId) -> AppResult<bool> {
    #[cfg(feature = "unstable-msc4495")]
    {
        return crate::user::presence::sharing::may_see(sender_id, viewer_id).await;
    }
    #[cfg(not(feature = "unstable-msc4495"))]
    {
        state::user_can_see_user(viewer_id, sender_id).await
    }
}
