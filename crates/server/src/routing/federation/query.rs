//! Endpoints to retrieve information from a homeserver about a resource.

use palpo_core::federation::query::ProfileReqArgs;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::federation::query::{ProfileResBody, RoomInfoResBody};
use crate::core::identifiers::*;
use crate::core::profile::ProfileFieldValue;
use crate::{
    AuthArgs, EmptyResult, IsRemoteOrLocal, JsonResult, MatrixError, config, data, json_ok,
};

#[cfg(feature = "unstable-msc4495")]
pub fn unstable_router() -> Router {
    Router::with_path("org.continuwuity.presence_v2.msc4495/query")
        .push(Router::with_path("presence_recipients").get(get_presence_recipients))
}

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
    if !config::get().federation.allow_inbound_profile_lookup {
        return Err(MatrixError::forbidden(
            "Profile lookup over federation is disabled on this homeserver",
            None,
        )
        .into());
    }

    if args.user_id.server_name().is_remote() {
        return Err(MatrixError::invalid_param("User does not belong to this server.").into());
    }

    let mut response = ProfileResBody::new();

    let profile = data::user::get_profile(&args.user_id, None)
        .await?
        .ok_or(MatrixError::not_found("Profile not found."))?;
    let custom_fields = profile.fields.as_object().cloned().unwrap_or_default();

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
        Some(field) => {
            if let Some(value) = custom_fields.get(field) {
                response.set(field, value.clone());
            }
        }
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
            response.extend(custom_fields);
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
    let room_id = crate::room::resolve_local_alias(&room_alias).await?;
    let mut servers = crate::room::lookup_servers(&room_id).await?;
    servers.insert(0, config::get().server_name.to_owned());
    servers.dedup();
    json_ok(RoomInfoResBody { room_id, servers })
}
#[endpoint]
async fn query_by_type(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::unrecognized("Unsupported federation query type.").into())
}

/// #GET /_matrix/federation/unstable/org.continuwuity.presence_v2.msc4495/query/presence_recipients
/// Returns a local user's current presence recipient set for the asking server ([MSC4495]).
///
/// A server whose view of the set has fallen out of step -- a delta whose `prev_id` it does
/// not hold -- calls this to resynchronise. Only the asking server's own users are
/// returned; the set for another server is none of its business, and the proposal scopes
/// the answer that way for exactly that reason.
///
/// [MSC4495]: https://github.com/matrix-org/matrix-spec-proposals/pull/4495
#[cfg(feature = "unstable-msc4495")]
#[endpoint]
pub(super) async fn get_presence_recipients(
    _aa: AuthArgs,
    args: crate::core::federation::query::PresenceRecipientsReqArgs,
    depot: &mut Depot,
) -> JsonResult<crate::core::federation::query::PresenceRecipientsResBody> {
    use crate::DepotExt;
    use crate::user::presence::{recipients, sharing};

    let origin = depot.origin()?.clone();

    if args.user_id.server_name().is_remote() {
        return Err(MatrixError::invalid_param("User does not belong to this server.").into());
    }

    let stream_id = recipients::stream_id(&args.user_id).await?;
    let recipients: Vec<_> = sharing::recipients_of(&args.user_id)
        .await?
        .into_iter()
        .filter(|recipient| recipient.server_name() == origin)
        .collect();

    if recipients.is_empty() {
        // The proposal answers 404 when there is no set for the asking server, which keeps
        // "shares with nobody here" distinguishable from "set is momentarily empty".
        return Err(MatrixError::not_found("No presence recipients for this server.").into());
    }

    // Record the snapshot so the next delta we send is computed against what the asking
    // server now holds, rather than against a view it has just discarded.
    recipients::record_confirmed(
        &args.user_id,
        &origin,
        stream_id,
        &recipients.iter().cloned().collect(),
    )
    .await?;

    json_ok(crate::core::federation::query::PresenceRecipientsResBody::new(stream_id, recipients))
}
