mod account;
mod admin;
mod appservice;
mod auth;
mod device;
mod directory;
mod key;
mod presence;
mod profile;
mod push_rule;
mod pusher;
mod register;
mod room;
mod room_key;
mod session;
pub mod sync_msc4186;
mod sync_v3;
mod third_party;
mod to_device;
mod unstable;
mod user;
mod user_directory;
mod voip;

pub(crate) mod media;

use std::collections::BTreeMap;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::config;
use crate::core::client::discovery::{
    Capabilities, CapabilitiesResBody, RoomVersionStability, RoomVersionsCapability,
    VersionsResBody,
};
use crate::core::client::search::{ResultCategories, SearchReqArgs, SearchReqBody, SearchResBody};
use crate::routing::prelude::*;

pub fn router() -> Router {
    let mut client = Router::with_path("client").oapi_tag("client");
    for v in ["v3", "v1", "r0"] {
        client = client
            .push(
                Router::with_path(v)
                    .push(account::public_router())
                    .push(profile::public_router())
                    .push(register::public_router())
                    .push(session::public_router())
                    .push(room::public_router())
                    .push(directory::public_router())
                    .push(media::self_auth_router())
                    .push(
                        Router::with_path("publicRooms")
                            .get(room::get_public_rooms)
                            .post(room::get_filtered_public_rooms),
                    ),
            )
            .push(
                Router::with_path(v)
                    .hoop(hoops::auth_by_access_token)
                    .push(account::authed_router())
                    .push(register::authed_router())
                    .push(session::authed_router())
                    .push(device::authed_router())
                    .push(room_key::authed_router())
                    .push(room::authed_router())
                    .push(user::authed_router())
                    .push(directory::authed_router())
                    .push(user_directory::authed_router())
                    .push(key::authed_router())
                    .push(profile::authed_router())
                    .push(voip::authed_router())
                    .push(appservice::authed_router())
                    .push(admin::authed_router())
                    .push(third_party::authed_router())
                    .push(to_device::authed_router())
                    .push(auth::authed_router())
                    .push(pusher::authed_router())
                    .push(push_rule::authed_router())
                    .push(presence::authed_router())
                    .push(Router::with_path("joined_rooms").get(room::membership::joined_rooms))
                    .push(
                        Router::with_path("join/{room_id_or_alias}")
                            .post(room::membership::join_room_by_id_or_alias),
                    )
                    .push(Router::with_path("createRoom").post(room::create_room))
                    .push(Router::with_path("notifications").get(get_notifications))
                    .push(Router::with_path("sync").get(sync_v3::sync_events_v3))
                    .push(
                        Router::with_path("dehydrated_device")
                            .get(device::dehydrated)
                            .put(device::upsert_dehydrated)
                            .delete(device::delete_dehydrated)
                            .push(
                                Router::with_path("{device_id}/events")
                                    .post(to_device::for_dehydrated),
                            ),
                    ),
            )
            .push(
                Router::with_path(v)
                    .hoop(hoops::limit_rate)
                    .hoop(hoops::auth_by_access_token)
                    .push(Router::with_path("search").post(search))
                    .push(Router::with_path("capabilities").get(get_capabilities))
                    .push(Router::with_path("knock/{room_id_or_alias}").post(room::knock_room)),
            )
    }
    client
        .push(Router::with_path("versions").get(supported_versions))
        .push(unstable::router())
}

/// #POST /_matrix/client/r0/search
/// Searches rooms for messages.
///
/// - Only works if the user is currently joined to the room (TODO: Respect history visibility)
#[endpoint]
fn search(
    _aa: AuthArgs,
    args: SearchReqArgs,
    body: JsonBody<SearchReqBody>,
    depot: &mut Depot,
) -> JsonResult<SearchResBody> {
    let authed = depot.authed_info()?;

    let search_criteria = body.search_categories.room_events.as_ref().unwrap();
    let room_events = crate::event::search::search_pdus(
        authed.user_id(),
        &search_criteria,
        args.next_batch.as_deref(),
    )?;
    json_ok(SearchResBody::new(ResultCategories { room_events }))
}

/// #GET /_matrix/client/r0/capabilities
/// Get information on the supported feature set and other relevent capabilities of this server.
#[endpoint]
async fn get_capabilities(_aa: AuthArgs) -> JsonResult<CapabilitiesResBody> {
    let mut available = BTreeMap::new();
    let conf = crate::config::get();
    for room_version in &*config::UNSTABLE_ROOM_VERSIONS {
        available.insert(room_version.clone(), RoomVersionStability::Unstable);
    }
    for room_version in &*config::STABLE_ROOM_VERSIONS {
        available.insert(room_version.clone(), RoomVersionStability::Stable);
    }
    json_ok(CapabilitiesResBody {
        capabilities: Capabilities {
            room_versions: RoomVersionsCapability {
                default: conf.default_room_version.clone(),
                available,
            },
            ..Default::default()
        },
    })
}

/// #GET /_matrix/client/versions
/// Get the versions of the specification and unstable features supported by this server.
///
/// - Versions take the form MAJOR.MINOR.PATCH
/// - Only the latest PATCH release will be reported for each MAJOR.MINOR value
/// - Unstable features are namespaced and may include version information in their name
///
/// Note: Unstable features are used while developing new features. Clients should avoid using
/// unstable features in their stable releases
#[endpoint]
async fn supported_versions() -> JsonResult<VersionsResBody> {
    json_ok(VersionsResBody {
        versions: vec![
            "r0.5.0".to_owned(),
            "r0.6.0".to_owned(),
            "v1.1".to_owned(),
            "v1.2".to_owned(),
            "v1.3".to_owned(),
            "v1.4".to_owned(),
            "v1.5".to_owned(),
            "v1.6".to_owned(),
            "v1.7".to_owned(),
            "v1.8".to_owned(),
            "v1.9".to_owned(),
            "v1.10".to_owned(),
            "v1.11".to_owned(),
        ],
        unstable_features: BTreeMap::from_iter([
            ("org.matrix.e2e_cross_signing".to_owned(), true),
            ("org.matrix.simplified_msc3575".to_owned(), true), /* Simplified Sliding sync (https://github.com/matrix-org/matrix-spec-proposals/pull/4186) */
        ]),
    })
}

#[endpoint]
async fn get_notifications(_aa: AuthArgs, depot: &mut Depot) -> EmptyResult {
    // TODO: get_notifications
    let _authed = depot.authed_info()?;
    empty_ok()
}
