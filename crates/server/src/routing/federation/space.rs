use futures_util::{FutureExt, StreamExt, stream};
use salvo::prelude::*;

use crate::core::MatrixError;
use crate::core::federation::space::{HierarchyReqArgs, HierarchyResBody};
use crate::room::space::{Identifier, SummaryAccessibility, get_parent_children_via};
use crate::{AuthArgs, DepotExt, JsonResult, json_ok};

pub fn router() -> Router {
    Router::with_path("hierarchy/{room_id}").get(get_hierarchy)
}

/// # `GET /_matrix/federation/v1/hierarchy/{room_id}`
///
/// Gets the space tree in a depth-first manner to locate child rooms of a given
/// space.
#[endpoint]
async fn get_hierarchy(
    _aa: AuthArgs,
    args: HierarchyReqArgs,
    depot: &mut Depot,
) -> JsonResult<HierarchyResBody> {
    if !crate::room::room_exists(&args.room_id).await? {
        return Err(MatrixError::not_found("Room does not exist.").into());
    }

    let origin = depot.origin()?;

    let room_id = &args.room_id;
    let suggested_only = args.suggested_only;
    let identifier = &Identifier::ServerName(origin);
    match crate::room::space::get_summary_and_children_local(room_id, identifier, suggested_only)
        .await?
    {
        None => Err(MatrixError::not_found("The requested room was not found").into()),

        Some(SummaryAccessibility::Inaccessible) => {
            Err(MatrixError::not_found("The requested room is inaccessible").into())
        }

        Some(SummaryAccessibility::Accessible(room)) => {
            let children_via = get_parent_children_via(&room, suggested_only);
            let (children, inaccessible_children) = stream::iter(children_via)
                .filter_map(|(child, _via)| async move {
                    match crate::room::space::get_summary_and_children_local(
                        &child,
                        identifier,
                        suggested_only,
                    )
                    .await
                    .ok()?
                    {
                        None => None,
                        Some(SummaryAccessibility::Inaccessible) => Some((None, Some(child))),
                        Some(SummaryAccessibility::Accessible(summary)) => {
                            Some((Some(summary), None))
                        }
                    }
                })
                .unzip()
                .map(|(children, inaccessible_children): (Vec<_>, Vec<_>)| {
                    (
                        children.into_iter().flatten().map(Into::into).collect(),
                        inaccessible_children.into_iter().flatten().collect(),
                    )
                })
                .await;

            json_ok(HierarchyResBody {
                room,
                children,
                inaccessible_children,
            })
        }
    }
}
