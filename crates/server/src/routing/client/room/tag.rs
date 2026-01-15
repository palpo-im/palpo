use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::tag::{OperateTagReqArgs, TagsResBody, UpsertTagReqBody};
use crate::core::events::RoomAccountDataEventType;
use crate::core::events::tag::TagEventContent;
use crate::core::user::UserRoomReqArgs;
use crate::{AuthArgs, DepotExt, EmptyResult, JsonResult, data, empty_ok, json_ok};

/// #GET /_matrix/client/r0/user/{user_id}/rooms/{room_idd}/tags
/// Returns tags on the room.
///
/// - Gets the tag event of the room account data.
#[endpoint]
pub(super) async fn list_tags(
    _aa: AuthArgs,
    args: UserRoomReqArgs,
    depot: &mut Depot,
) -> JsonResult<TagsResBody> {
    let authed = depot.authed_info()?;

    let user_data_content = data::user::get_data::<TagEventContent>(
        authed.user_id(),
        Some(&args.room_id),
        &RoomAccountDataEventType::Tag.to_string(),
    )
    .unwrap_or_default();

    json_ok(TagsResBody {
        tags: user_data_content.tags,
    })
}

/// #PUT /_matrix/client/r0/user/{user_id}/rooms/{room_id}/tags/{tag}
/// Adds a tag to the room.
///
/// - Inserts the tag into the tag event of the room account data.
#[endpoint]
pub(super) async fn upsert_tag(
    _aa: AuthArgs,
    args: OperateTagReqArgs,
    body: JsonBody<UpsertTagReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    let mut user_data_content = data::user::get_data::<TagEventContent>(
        authed.user_id(),
        Some(&args.room_id),
        &RoomAccountDataEventType::Tag.to_string(),
    )
    .unwrap_or_default();

    user_data_content
        .tags
        .insert(args.tag.clone().into(), body.tag_info.clone());

    crate::data::user::set_data(
        authed.user_id(),
        Some(args.room_id.clone()),
        &RoomAccountDataEventType::Tag.to_string(),
        serde_json::to_value(user_data_content).expect("to json value always works"),
    )?;
    empty_ok()
}

/// #DELETE /_matrix/client/r0/user/{user_id}/rooms/{room_id}/tags/{tag}
/// Deletes a tag from the room.
///
/// - Removes the tag from the tag event of the room account data.
#[endpoint]
pub(super) async fn delete_tag(
    _aa: AuthArgs,
    args: OperateTagReqArgs,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    let mut user_data_content = crate::data::user::get_data::<TagEventContent>(
        authed.user_id(),
        Some(&args.room_id),
        &RoomAccountDataEventType::Tag.to_string(),
    )
    .unwrap_or_default();

    user_data_content.tags.remove(&args.tag.clone().into());

    crate::data::user::set_data(
        authed.user_id(),
        Some(args.room_id.clone()),
        &RoomAccountDataEventType::Tag.to_string(),
        serde_json::to_value(user_data_content).expect("to json value always works"),
    )?;
    empty_ok()
}
