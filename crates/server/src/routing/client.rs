mod account;
mod admin;
mod appservice;
mod auth;
mod device;
mod directory;
mod key;
mod oidc;
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
use crate::core::client::discovery::capabilities::{
    Capabilities, CapabilitiesResBody, ChangePasswordCapability, ProfileFieldsCapability,
    RoomVersionStability, RoomVersionsCapability, ThirdPartyIdChangesCapability,
};
use crate::core::client::discovery::versions::VersionsResBody;
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
        .push(
            Router::with_path("oidc")
                .push(Router::with_path("status").get(oidc::oidc_status))
                .push(Router::with_path("auth").get(oidc::oidc_auth))
                .push(Router::with_path("callback").get(oidc::oidc_callback))
                .push(Router::with_path("login").post(oidc::oidc_login)),
        )
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
        search_criteria,
        args.next_batch.as_deref(),
    )?;
    json_ok(SearchResBody::new(ResultCategories { room_events }))
}

/// #GET /_matrix/client/r0/capabilities
/// Get information on the supported feature set and other relevent capabilities of this server.
#[endpoint]
fn get_capabilities(_aa: AuthArgs) -> JsonResult<CapabilitiesResBody> {
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
            // TODO: use config values
            change_password: ChangePasswordCapability { enabled: true },
            thirdparty_id_changes: ThirdPartyIdChangesCapability { enabled: true },
            profile_fields: Some(ProfileFieldsCapability::new(true)),
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
fn supported_versions() -> JsonResult<VersionsResBody> {
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
            "v1.12".to_owned(),
        ],
        unstable_features: BTreeMap::from_iter([
            ("org.matrix.e2e_cross_signing".to_owned(), true),
            ("org.matrix.msc2285.stable".to_owned(), true), /* private read receipts (https://github.com/matrix-org/matrix-spec-proposals/pull/2285) */
            ("uk.half-shot.msc2666.query_mutual_rooms".to_owned(), true), /* query mutual rooms (https://github.com/matrix-org/matrix-spec-proposals/pull/2666) */
            ("org.matrix.msc2836".to_owned(), true), /* threading/threads (https://github.com/matrix-org/matrix-spec-proposals/pull/2836) */
            ("org.matrix.msc2946".to_owned(), true), /* spaces/hierarchy summaries (https://github.com/matrix-org/matrix-spec-proposals/pull/2946) */
            ("org.matrix.msc3026.busy_presence".to_owned(), true), /* busy presence status (https://github.com/matrix-org/matrix-spec-proposals/pull/3026) */
            ("org.matrix.msc3827".to_owned(), true), /* filtering of /publicRooms by room type (https://github.com/matrix-org/matrix-spec-proposals/pull/3827) */
            ("org.matrix.msc3952_intentional_mentions".to_owned(), true), /* intentional mentions (https://github.com/matrix-org/matrix-spec-proposals/pull/3952) */
            ("org.matrix.msc3575".to_owned(), true), /* sliding sync (https://github.com/matrix-org/matrix-spec-proposals/pull/3575/files#r1588877046) */
            ("org.matrix.msc3916.stable".to_owned(), true), /* authenticated media (https://github.com/matrix-org/matrix-spec-proposals/pull/3916) */
            ("org.matrix.msc4180".to_owned(), true), /* stable flag for 3916 (https://github.com/matrix-org/matrix-spec-proposals/pull/4180) */
            ("uk.tcpip.msc4133".to_owned(), true), /* Extending User Profile API with Key:Value Pairs (https://github.com/matrix-org/matrix-spec-proposals/pull/4133) */
            ("us.cloke.msc4175".to_owned(), true), /* Profile field for user time zone (https://github.com/matrix-org/matrix-spec-proposals/pull/4175) */
            ("org.matrix.simplified_msc3575".to_owned(), true), /* Simplified Sliding sync (https://github.com/matrix-org/matrix-spec-proposals/pull/4186) */
        ]),
    })
}

/// #GET /_matrix/client/v3/notifications
/// Paginate through the list of events that the user has been notified about.
#[endpoint]
fn get_notifications(
    _aa: AuthArgs,
    from: QueryParam<String, false>,
    limit: QueryParam<usize, false>,
    only: QueryParam<String, false>,
    depot: &mut Depot,
) -> JsonResult<crate::core::client::push::NotificationsResBody> {
    use crate::core::client::push::{Notification, NotificationsResBody};

    let authed = depot.authed_info()?;

    let from_id: Option<i64> = from.into_inner().and_then(|s| s.parse().ok());
    let page_limit = limit.into_inner().unwrap_or(20).min(100) as i64;
    let only_highlight = only.into_inner().as_deref() == Some("highlight");

    let push_actions = crate::data::room::event::get_push_actions_for_user(
        authed.user_id(),
        from_id,
        page_limit,
        only_highlight,
    )?;

    let mut notifications = Vec::with_capacity(push_actions.len());
    let mut next_token: Option<String> = None;

    for action_row in &push_actions {
        next_token = Some(action_row.id.to_string());

        // Try to load the event PDU
        let pdu = match crate::room::timeline::get_pdu(&action_row.event_id) {
            Ok(pdu) => pdu,
            Err(_) => continue,
        };

        // Check if the user has read this event
        let read = crate::data::room::event::has_user_read_event(
            authed.user_id(),
            &action_row.room_id,
            action_row.event_sn,
        );

        // Deserialize stored actions
        let actions: Vec<crate::core::push::Action> =
            serde_json::from_value(action_row.actions.clone()).unwrap_or_default();

        notifications.push(Notification {
            actions,
            event: pdu.to_sync_room_event(),
            profile_tag: if action_row.profile_tag.is_empty() {
                None
            } else {
                Some(action_row.profile_tag.clone())
            },
            read,
            room_id: action_row.room_id.clone(),
            ts: pdu.origin_server_ts,
        });
    }

    let mut res = NotificationsResBody::new(notifications);
    if push_actions.len() == page_limit as usize {
        res.next_token = next_token;
    }
    json_ok(res)
}
