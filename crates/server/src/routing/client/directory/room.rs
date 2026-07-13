use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::oapi::extract::{JsonBody, PathParam};
use salvo::prelude::*;

use crate::core::client::directory::{SetRoomVisibilityReqBody, VisibilityResBody};
use crate::core::events::StateEventType;
use crate::core::identifiers::*;
use crate::core::room::Visibility;
use crate::data::room::DbRoom;
use crate::data::schema::*;
use crate::data::user::DbUser;
use crate::data::{connect, diesel_exists};
use crate::{
    AppError, AppResult, AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, config,
    empty_ok, json_ok,
};

/// #GET /_matrix/client/r0/directory/list/room/{room_id}
/// Gets the visibility of a given room in the room directory.
#[endpoint]
pub(super) async fn get_visibility(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
) -> JsonResult<VisibilityResBody> {
    let room_id = room_id.into_inner();
    let query = rooms::table
        .filter(rooms::id.eq(&room_id))
        .filter(rooms::is_public.eq(true));
    let visibility = if diesel_exists!(query, &mut connect().await?)? {
        Visibility::Public
    } else {
        Visibility::Private
    };

    json_ok(VisibilityResBody { visibility })
}
/// #PUT /_matrix/client/r0/directory/list/room/{room_id}
/// Sets the visibility of a given room in the room directory.
#[endpoint]
pub(super) async fn set_visibility(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<SetRoomVisibilityReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let room_id = room_id.into_inner();
    let room = rooms::table
        .find(&room_id)
        .first::<DbRoom>(&mut connect().await?)
        .await?;

    let visibility_is_public = body.visibility == Visibility::Public;
    if visibility_is_public && config::get().lockdown_public_room_directory && !authed.is_admin() {
        return Err(MatrixError::forbidden(
            "Only server admins can publish rooms to the room directory.",
            None,
        )
        .into());
    }

    if !user_can_change_visibility(authed.user(), &room_id).await? {
        return Err(MatrixError::forbidden(
            "User is not permitted to change this room's directory visibility.",
            None,
        )
        .into());
    }

    diesel::update(&room)
        .set(rooms::is_public.eq(visibility_is_public))
        .execute(&mut connect().await?)
        .await?;
    empty_ok()
}

#[endpoint]
pub(super) async fn set_visibility_with_network_id(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::unrecognized(
        "Appservice network room directory visibility is not implemented.",
    )
    .into())
}

async fn user_can_change_visibility(user: &DbUser, room_id: &RoomId) -> AppResult<bool> {
    if user.is_admin || config::server_user_id() == user.id {
        return Ok(true);
    }

    if !crate::room::user::is_joined(&user.id, room_id).await? {
        return Ok(false);
    }

    if let Ok(power_levels) = crate::room::get_power_levels(room_id).await {
        return Ok(power_levels.user_can_send_state(&user.id, StateEventType::RoomCanonicalAlias));
    }

    if let Ok(event) = crate::room::get_state(room_id, &StateEventType::RoomCreate, "", None).await
    {
        return Ok(event.sender == user.id);
    }

    error!("Room {} has no m.room.create event", room_id);
    Err(AppError::public("Room has no m.room.create event"))
}
