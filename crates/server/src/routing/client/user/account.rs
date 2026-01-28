use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Deserialize;

use crate::core::client::account::data::{GlobalAccountDataResBody, RoomAccountDataResBody};
use crate::core::events::AnyGlobalAccountDataEventContent;
use crate::core::identifiers::*;
use crate::core::serde::{JsonValue, RawJson};
use crate::core::user::{UserEventTypeReqArgs, UserRoomEventTypeReqArgs};
use crate::{AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, data, empty_ok, json_ok};

#[derive(Deserialize)]
struct ExtractGlobalEventContent {
    content: RawJson<AnyGlobalAccountDataEventContent>,
}

/// #GET /_matrix/client/r0/user/{user_id}/account_data/{event_type}
/// Gets some account data for the sender user.
#[endpoint]
pub(super) async fn get_global_data(
    _aa: AuthArgs,
    args: UserEventTypeReqArgs,
    depot: &mut Depot,
) -> JsonResult<GlobalAccountDataResBody> {
    let authed = depot.authed_info()?;

    let content =
        data::user::get_data::<JsonValue>(authed.user_id(), None, &args.event_type.to_string())
            .map_err(|_| MatrixError::not_found("user data not found"))?;

    json_ok(GlobalAccountDataResBody(RawJson::from_value(&content)?))
}

/// #PUT /_matrix/client/r0/user/{user_id}/account_data/{event_type}
/// Sets some account data for the sender user.
#[endpoint]
pub(super) async fn set_global_data(
    _aa: AuthArgs,
    args: UserEventTypeReqArgs,
    body: JsonBody<JsonValue>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let event_type = args.event_type.to_string();
    let body = body.into_inner();

    if event_type == "m.ignored_user_list" {
        let ignored_ids: Vec<OwnedUserId> = body
            .get("ignored_users")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(id, _)| OwnedUserId::try_from(id).ok())
            .collect();

        data::user::set_ignored_users(authed.user_id(), &ignored_ids)?;
    }

    data::user::set_data(authed.user_id(), None, &event_type, body)?;
    empty_ok()
}

/// #GET /_matrix/client/r0/user/{user_id}/rooms/{roomId}/account_data/{event_type}
/// Gets some account data for the sender user.
#[endpoint]
pub(super) async fn get_room_data(
    _aa: AuthArgs,
    args: UserRoomEventTypeReqArgs,
    depot: &mut Depot,
) -> JsonResult<RoomAccountDataResBody> {
    let authed = depot.authed_info()?;

    let content = data::user::get_data::<JsonValue>(
        authed.user_id(),
        Some(&*args.room_id),
        &args.event_type.to_string(),
    )
    .map_err(|_| MatrixError::not_found("user data not found"))?;

    json_ok(RoomAccountDataResBody(RawJson::from_value(&content)?))
}

/// #PUT /_matrix/client/r0/user/{user_id}/account_data/{event_type}
/// Sets some room account data for the sender user.
#[endpoint]
pub(super) async fn set_room_data(
    _aa: AuthArgs,
    args: UserRoomEventTypeReqArgs,
    body: JsonBody<JsonValue>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    let event_type = args.event_type.to_string();

    data::user::set_data(
        authed.user_id(),
        Some(args.room_id),
        &event_type,
        body.into_inner(),
    )?;
    empty_ok()
}
