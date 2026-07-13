use salvo::prelude::*;

use crate::core::client::relation::{
    RelatingEventsReqArgs, RelatingEventsWithRelTypeAndEventTypeReqArgs,
    RelatingEventsWithRelTypeReqArgs, RelationEventsResBody,
};
use crate::{AuthArgs, DepotExt, JsonResult, json_ok};

/// #GET /_matrix/client/r0/rooms/{room_id}/relations/{event_id}
#[endpoint]
pub(super) async fn get_relation(
    _aa: AuthArgs,
    args: RelatingEventsReqArgs,
    depot: &mut Depot,
) -> JsonResult<RelationEventsResBody> {
    let authed = depot.authed_info()?;

    let body = crate::room::pdu_metadata::paginate_relations_with_filter(
        authed.user_id(),
        &args.room_id,
        &args.event_id,
        None,
        None,
        args.from.as_deref(),
        args.to.as_deref(),
        args.limit,
        args.recurse,
        args.dir,
    )
    .await?;
    json_ok(body)
}

/// #GET /_matrix/client/r0/rooms/{room_id}/relations/{event_id}/{rel_type}
#[endpoint]
pub(super) async fn get_relation_by_rel_type(
    _aa: AuthArgs,
    args: RelatingEventsWithRelTypeReqArgs,
    depot: &mut Depot,
) -> JsonResult<RelationEventsResBody> {
    let authed = depot.authed_info()?;

    let body = crate::room::pdu_metadata::paginate_relations_with_filter(
        authed.user_id(),
        &args.room_id,
        &args.event_id,
        None,
        Some(args.rel_type.clone()),
        args.from.as_deref(),
        args.to.as_deref(),
        args.limit,
        args.recurse,
        args.dir,
    )
    .await?;

    json_ok(body)
}

/// #GET /_matrix/client/r0/rooms/{room_id}/relations/{event_id}/{rel_type}/{event_type}
#[endpoint]
pub(super) async fn get_relation_by_rel_type_and_event_type(
    _aa: AuthArgs,
    args: RelatingEventsWithRelTypeAndEventTypeReqArgs,
    depot: &mut Depot,
) -> JsonResult<RelationEventsResBody> {
    let authed = depot.authed_info()?;

    let body = crate::room::pdu_metadata::paginate_relations_with_filter(
        authed.user_id(),
        &args.room_id,
        &args.event_id,
        Some(args.event_type.clone()),
        Some(args.rel_type.clone()),
        args.from.as_deref(),
        args.to.as_deref(),
        args.limit,
        args.recurse,
        args.dir,
    )
    .await?;

    json_ok(body)
}
