use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::oapi::extract::{JsonBody, PathParam};
use salvo::prelude::*;

use crate::core::client::room::{AliasResBody, SetAliasReqBody};
use crate::core::identifiers::*;
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::exts::*;
use crate::{AuthArgs, EmptyResult, JsonResult, MatrixError, empty_ok, json_ok};

/// #GET /_matrix/client/r0/directory/room/{room_alias}
/// Resolve an alias locally or over federation.
///
/// - TODO: Suggest more servers to join via
#[endpoint]
pub(super) async fn get_alias(
    _aa: AuthArgs,
    room_alias: PathParam<OwnedRoomAliasId>,
) -> JsonResult<AliasResBody> {
    let room_alias = room_alias.into_inner();
    let (room_id, servers) = match crate::room::resolve_alias(&room_alias, None).await {
        Ok(resolved) => resolved,
        // A genuinely unknown alias is reported as a clean `M_NOT_FOUND`, but any other
        // failure (federation transport, signature rejection, internal error, ...) is
        // propagated as-is so it stays visible to the client and in the logs instead of
        // being masked as "room does not exist".
        Err(e) if e.is_not_found() => {
            return Err(MatrixError::not_found("Room with alias not found.").into());
        }
        Err(e) => return Err(e),
    };
    let servers = crate::room::room_available_servers(&room_id, &room_alias, servers).await?;
    debug!(?room_alias, ?room_id, "available servers: {servers:?}");
    json_ok(AliasResBody::new(room_id, servers))
}

/// #PUT /_matrix/client/r0/directory/room/{room_alias}
/// Creates a new room alias on this server.
#[endpoint]
pub(super) async fn upsert_alias(
    _aa: AuthArgs,
    room_alias: PathParam<OwnedRoomAliasId>,
    body: JsonBody<SetAliasReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let alias_id = room_alias.into_inner();
    if alias_id.is_remote() {
        return Err(MatrixError::invalid_param("alias is from another server").into());
    }

    if crate::room::resolve_local_alias(&alias_id).await.is_ok() {
        return Err(MatrixError::forbidden("alias already exists", None).into());
    }

    let query = room_aliases::table
        .filter(room_aliases::alias_id.eq(&alias_id))
        .filter(room_aliases::room_id.ne(&body.room_id));
    if diesel_exists!(query, &mut connect().await?)? {
        return Err(StatusError::conflict()
            .brief("a room alias with that name already exists")
            .into());
    }

    crate::room::set_alias(body.room_id.clone(), alias_id, authed.user_id()).await?;

    empty_ok()
}

/// #DELETE /_matrix/client/r0/directory/room/{room_alias}
/// Deletes a room alias from this server.
///
/// - TODO: additional access control checks
/// - TODO: Update canonical alias event
#[endpoint]
pub(super) async fn delete_alias(
    _aa: AuthArgs,
    room_alias: PathParam<OwnedRoomAliasId>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    let alias = room_alias.into_inner();
    if alias.is_remote() {
        return Err(MatrixError::invalid_param("Alias is from another server.").into());
    }

    crate::room::remove_alias(&alias, authed.user()).await?;

    // TODO: update alt_aliases?

    empty_ok()
}
