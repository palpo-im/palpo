//! Endpoints to retrieve information from a homeserver about a resource.

use palpo_core::federation::query::ProfileReqArgs;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::federation::query::{ProfileResBody, RoomInfoResBody};
use crate::core::identifiers::*;
use crate::core::profile::ProfileFieldValue;
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

/// #GET /_matrix/federation/v1/query/profile
/// Gets information on a profile.
#[endpoint]
async fn get_profile(_aa: AuthArgs, args: ProfileReqArgs) -> JsonResult<ProfileResBody> {
    if args.user_id.server_name().is_remote() {
        return Err(MatrixError::invalid_param("User does not belong to this server.").into());
    }

    let mut response = ProfileResBody::new();

    let profile = data::user::get_profile(&args.user_id, None)?
        .ok_or(MatrixError::not_found("Profile not found."))?;

    match args.field.as_ref().map(|field| field.as_str()) {
        Some("displayname") => {
            if let Some(display_name) = profile.display_name {
                response.extend([ProfileFieldValue::DisplayName(display_name)]);
            }
        }
        Some("avatar_url") => {
            if let Some(avatar_url) = profile.avatar_url {
                response.extend([ProfileFieldValue::AvatarUrl(avatar_url)]);
            }
            if let Some(blurhash) = profile.blurhash {
                response.set("xyz.amorgan.blurhash", blurhash.into());
            }
        }
        Some("xyz.amorgan.blurhash") => {
            if let Some(blurhash) = profile.blurhash {
                response.set("xyz.amorgan.blurhash", blurhash.into());
            }
        }
        Some(_) => {}
        None => {
            if let Some(display_name) = profile.display_name {
                response.extend([ProfileFieldValue::DisplayName(display_name)]);
            }
            if let Some(avatar_url) = profile.avatar_url {
                response.extend([ProfileFieldValue::AvatarUrl(avatar_url)]);
            }
            if let Some(blurhash) = profile.blurhash {
                response.set("xyz.amorgan.blurhash", blurhash.into());
            }
        }
    }

    json_ok(response)
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
