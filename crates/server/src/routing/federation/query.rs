//! Endpoints to retrieve information from a homeserver about a resource.

use palpo_core::federation::query::ProfileReqArgs;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::federation::query::{EduTypesResBody, RoomInfoResBody};
use crate::core::identifiers::*;
use crate::core::user::{ProfileField, ProfileResBody};
use crate::{
    AuthArgs, EmptyResult, IsRemoteOrLocal, JsonResult, MatrixError, config, data, empty_ok,
    json_ok,
};

pub fn router() -> Router {
    Router::with_path("query")
        .push(Router::with_path("profile").get(get_profile))
        .push(Router::with_path("directory").get(get_directory))
        .push(Router::with_path("{query_type}").get(query_by_type))
}

/// #GET /_matrix/federation/unstable/io.fsky.vel/edutypes
/// Determine what types of EDUs this server wishes to receive.
#[endpoint]
pub async fn get_edu_types() -> JsonResult<EduTypesResBody> {
    let conf = config::get();

    json_ok(EduTypesResBody {
        presence: conf.presence.allow_incoming,
        receipt: conf.read_receipt.allow_incoming,
        typing: conf.typing.allow_incoming,
    })
}

/// #GET /_matrix/federation/v1/query/profile
/// Gets information on a profile.
#[endpoint]
async fn get_profile(_aa: AuthArgs, args: ProfileReqArgs) -> JsonResult<ProfileResBody> {
    if args.user_id.server_name().is_remote() {
        return Err(MatrixError::invalid_param("User does not belong to this server.").into());
    }

    let mut display_name = None;
    let mut avatar_url = None;
    let mut blurhash = None;

    let profile = data::user::get_profile(&args.user_id, None)?
        .ok_or(MatrixError::not_found("Profile not found."))?;

    match &args.field {
        Some(ProfileField::DisplayName) => display_name = profile.display_name.clone(),
        Some(ProfileField::AvatarUrl) => {
            avatar_url = profile.avatar_url.clone();
            blurhash = profile.blurhash.clone();
        }
        // TODO: what to do with custom
        Some(_) => {}
        None => {
            display_name = profile.display_name.clone();
            avatar_url = profile.avatar_url.clone();
            blurhash = profile.blurhash.clone();
        }
    }

    json_ok(ProfileResBody {
        blurhash,
        display_name,
        avatar_url,
    })
}

/// #GET /_matrix/federation/v1/query/directory
/// Resolve a room alias to a room id.
#[endpoint]
async fn get_directory(
    _aa: AuthArgs,
    room_alias: QueryParam<OwnedRoomAliasId, true>,
) -> JsonResult<RoomInfoResBody> {
    let room_id = crate::room::resolve_local_alias(&room_alias)?;
    let mut servers = crate::room::lookup_servers(&room_id)?;
    servers.insert(0, config::get().server_name.to_owned());
    servers.dedup();
    json_ok(RoomInfoResBody { room_id, servers })
}
#[endpoint]
async fn query_by_type(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
}
