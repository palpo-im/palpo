mod event;
pub(super) mod membership;
mod message;
mod receipt;
mod relation;
mod space;
mod state;
pub mod summary;
mod tag;
mod thread;
use std::cmp::max;
use std::collections::BTreeMap;

pub(crate) use membership::knock_room;
use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde_json::json;
use serde_json::value::to_raw_value;
use ulid::Ulid;

use crate::core::UnixMillis;
use crate::core::client::directory::{PublicRoomsFilteredReqBody, PublicRoomsReqArgs};
use crate::core::client::room::{
    AliasesResBody, CreateRoomReqBody, CreateRoomResBody, CreationContent, InitialSyncReqArgs,
    InitialSyncResBody, PaginationChunk, RoomPreset, SetReadMarkerReqBody, UpgradeRoomReqBody,
    UpgradeRoomResBody,
};
use crate::core::directory::{PublicRoomFilter, PublicRoomsResBody, RoomNetwork};
use crate::core::events::fully_read::{FullyReadEvent, FullyReadEventContent};
use crate::core::events::receipt::{
    Receipt, ReceiptEvent, ReceiptEventContent, ReceiptThread, ReceiptType,
};
use crate::core::events::room::canonical_alias::RoomCanonicalAliasEventContent;
use crate::core::events::room::create::RoomCreateEventContent;
use crate::core::events::room::guest_access::{GuestAccess, RoomGuestAccessEventContent};
use crate::core::events::room::history_visibility::{
    HistoryVisibility, RoomHistoryVisibilityEventContent,
};
use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
use crate::core::events::room::member::{MembershipState, RoomMemberEventContent};
use crate::core::events::room::name::RoomNameEventContent;
use crate::core::events::room::power_levels::RoomPowerLevelsEventContent;
use crate::core::events::room::tombstone::RoomTombstoneEventContent;
use crate::core::events::room::topic::RoomTopicEventContent;
use crate::core::events::{self, RoomAccountDataEventType, StateEventType, TimelineEventType};
use crate::core::identifiers::*;
use crate::core::room::{JoinRule, Visibility};
use crate::core::room_version_rules::{AuthorizationRules, RoomIdFormatVersion, RoomVersionRules};
use crate::core::serde::{CanonicalJsonObject, JsonValue, RawJson};
use crate::core::state::events::RoomCreateEvent;
use crate::event::PduBuilder;
use crate::room::{push_action, timeline};
use crate::user::user_is_ignored;
use crate::{
    AppResult, AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, RoomMutexGuard, config,
    data, empty_ok, hoops, json_ok, room,
};

const LIMIT_MAX: usize = 100;

pub fn public_router() -> Router {
    Router::with_path("rooms").push(
        Router::with_path("{room_id}").push(Router::with_path("initialSync").get(initial_sync)),
    )
}
pub fn authed_router() -> Router {
    Router::with_path("rooms")
        .push(
            Router::with_hoop(hoops::limit_rate).push(
                Router::with_path("{room_id}")
                    .push(Router::with_path("forget").post(membership::forget_room))
                    .push(Router::with_path("leave").post(membership::leave_room))
                    .push(Router::with_path("join").post(membership::join_room_by_id))
                    .push(Router::with_path("invite").post(membership::invite_user))
                    .push(Router::with_path("read_markers").post(set_read_markers))
                    .push(Router::with_path("aliases").get(get_aliases))
                    .push(Router::with_path("hierarchy").get(space::get_hierarchy))
                    .push(Router::with_path("threads").get(thread::list_threads))
                    .push(Router::with_path("typing/{user_id}").put(state::send_typing))
                    .push(
                        Router::with_path("receipt/{receipt_type}/{event_id}")
                            .post(receipt::send_receipt)
                            .put(receipt::send_receipt),
                    )
                    .push(Router::with_path("timestamp_to_event").get(event::timestamp_to_event)),
            ),
        )
        .push(
            Router::with_hoop(hoops::limit_rate).push(
                Router::with_path("{room_id}")
                    .push(Router::with_path("ban").post(membership::ban_user))
                    .push(Router::with_path("unban").post(membership::unban_user))
                    .push(Router::with_path("kick").post(membership::kick_user))
                    .push(Router::with_path("members").get(membership::get_members))
                    .push(Router::with_path("joined_members").get(membership::joined_members))
                    .push(
                        Router::with_path("state").get(state::get_state).push(
                            Router::with_path("{event_type}")
                                .put(state::send_state_for_empty_key)
                                .get(state::state_for_empty_key)
                                .push(
                                    Router::with_path("{state_key}")
                                        .put(state::send_state_for_key)
                                        .get(state::state_for_key),
                                ),
                        ),
                    )
                    .push(
                        Router::with_path("context")
                            .push(Router::with_path("{event_id}").get(event::get_context)),
                    )
                    .push(
                        Router::with_path("relations").push(
                            Router::with_path("{event_id}")
                                .get(relation::get_relation)
                                .push(
                                    Router::with_path("{rel_type}")
                                        .get(relation::get_relation_by_rel_type)
                                        .push(Router::with_path("{event_type}").get(
                                            relation::get_relation_by_rel_type_and_event_type,
                                        )),
                                ),
                        ),
                    )
                    .push(Router::with_path("upgrade").post(upgrade))
                    .push(Router::with_path("messages").get(message::get_messages))
                    .push(Router::with_path("send/{event_type}").post(message::post_message))
                    .push(
                        Router::with_path("send/{event_type}/{txn_id}").put(message::send_message),
                    )
                    .push(Router::with_path("redact/{event_id}/{txn_id}").put(event::send_redact))
                    .push(
                        Router::with_path("tags").get(tag::list_tags).push(
                            Router::with_path("{tag}")
                                .put(tag::upsert_tag)
                                .delete(tag::delete_tag),
                        ),
                    )
                    .push(
                        Router::with_path("event").push(
                            Router::with_path("{event_id}")
                                .get(event::get_room_event)
                                .post(event::report),
                        ),
                    ),
            ),
        )
}

// `#GET /_matrix/client/r0/rooms/{room_id}/initialSync`
#[endpoint]
async fn initial_sync(
    _aa: AuthArgs,
    args: InitialSyncReqArgs,
    depot: &mut Depot,
) -> JsonResult<InitialSyncResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let room_id = &args.room_id;

    if !room::state::user_can_see_events(sender_id, room_id)? {
        return Err(MatrixError::forbidden("No room preview available.", None).into());
    }

    let limit = LIMIT_MAX;
    let events =
        timeline::stream::load_pdus_backward(Some(sender_id), room_id, None, None, None, limit)?;

    let frame_id = room::get_frame_id(room_id, None).unwrap_or_default();
    let state: Vec<_> = room::state::get_full_state(frame_id)
        .unwrap_or_default()
        .into_values()
        .map(|event| event.to_state_event())
        .collect::<Vec<_>>();

    let messages = PaginationChunk {
        start: events
            .last()
            .map(|(sn, _)| sn)
            .as_ref()
            .map(ToString::to_string),
        end: events
            .first()
            .map(|(sn, _)| sn)
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        chunk: events
            .into_iter()
            .map(|(_sn, event)| event.to_room_event())
            .collect(),
    };

    json_ok(InitialSyncResBody {
        room_id: room_id.to_owned(),
        account_data: None,
        state: state.into(),
        messages: if !messages.chunk.is_empty() {
            Some(messages)
        } else {
            None
        },
        visibility: room::directory::visibility(room_id).into(),
        membership: room::user::membership(sender_id, room_id).ok(),
    })
}
/// `#POST /_matrix/client/r0/rooms/{room_id}/read_markers`
/// Sets different types of read markers.
///
/// - Updates fully-read account data event to `fully_read`
/// - If `read_receipt` is set: Update private marker and public read receipt EDU
#[endpoint]
fn set_read_markers(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<SetReadMarkerReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let room_id = room_id.into_inner();
    if let Some(fully_read) = &body.fully_read {
        let fully_read_event = FullyReadEvent {
            content: FullyReadEventContent {
                event_id: fully_read.clone(),
            },
        };
        crate::data::user::set_data(
            sender_id,
            Some(room_id.clone()),
            &RoomAccountDataEventType::FullyRead.to_string(),
            serde_json::to_value(fully_read_event.content).expect("to json value always works"),
        )?;
        push_action::remove_actions_for_room(sender_id, &room_id)?;
    }

    if let Some(event_id) = &body.private_read_receipt {
        let (event_sn, _event_guard) = crate::event::ensure_event_sn(&room_id, event_id)?;
        data::room::receipt::set_private_read(&room_id, sender_id, event_id, event_sn)?;
        push_action::remove_actions_until(sender_id, &room_id, event_sn, None)?;
        push_action::refresh_notify_summary(sender_id, &room_id)?;
    }

    if let Some(event) = &body.read_receipt {
        let mut user_receipts = BTreeMap::new();
        user_receipts.insert(
            sender_id.to_owned(),
            Receipt {
                ts: Some(UnixMillis::now()),
                thread: ReceiptThread::Unthreaded,
            },
        );

        let mut receipts = BTreeMap::new();
        receipts.insert(ReceiptType::Read, user_receipts);

        let mut receipt_content = BTreeMap::new();
        receipt_content.insert(event.to_owned(), receipts);

        room::receipt::update_read(
            sender_id,
            &room_id,
            &ReceiptEvent {
                content: ReceiptEventContent(receipt_content),
                room_id: room_id.clone(),
            },
            true,
        )?;
        let event_sn = crate::event::get_event_sn(event)?;
        push_action::remove_actions_until(sender_id, &room_id, event_sn, None)?;
        push_action::refresh_notify_summary(sender_id, &room_id)?;
    }
    empty_ok()
}

/// #GET /_matrix/client/r0/rooms/{room_id}/aliases
/// Lists all aliases of the room.
///
/// - Only users joined to the room are allowed to call this
/// TODO: Allow any user to call it if history_visibility is world readable
#[endpoint]
async fn get_aliases(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    depot: &mut Depot,
) -> JsonResult<AliasesResBody> {
    let authed = depot.authed_info()?;

    if !room::user::is_joined(authed.user_id(), &room_id)? {
        return Err(
            MatrixError::forbidden("you don't have permission to view this room", None).into(),
        );
    }

    json_ok(AliasesResBody {
        aliases: room::local_aliases_for_room(&room_id)?,
    })
}

/// #POST /_matrix/client/r0/rooms/{room_id}/upgrade
/// Upgrades the room.
///
/// - Creates a replacement room
/// - Sends a tombstone event into the current room
/// - Sender user joins the room
/// - Transfers some state events
/// - Moves local aliases
/// - Modifies old room power levels to prevent users from speaking
#[endpoint]
async fn upgrade(
    _aa: AuthArgs,
    room_id: PathParam<OwnedRoomId>,
    body: JsonBody<UpgradeRoomReqBody>,
    depot: &mut Depot,
) -> JsonResult<UpgradeRoomResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let room_id = room_id.into_inner();

    if !config::supported_room_versions().contains(&body.new_version) {
        return Err(MatrixError::unsupported_room_version(
            "This server does not support that room version.",
        )
        .into());
    }

    let conf = config::get();
    let version_rules = crate::room::get_version_rules(&body.new_version)?;

    // Create a replacement room
    let new_room_id = if version_rules.authorization.room_create_event_id_as_room_id {
        OwnedRoomId::try_from(format!("!placehold_{}", Ulid::new().to_string()))
            .expect("room id should be valid")
    } else {
        RoomId::new_v1(&conf.server_name)
    };

    let state_lock = room::lock_state(&room_id).await;
    room::ensure_room(&new_room_id, &body.new_version)?;

    // Use the m.room.tombstone event as the predecessor
    let predecessor = Some(events::room::create::PreviousRoom::new(room_id.clone()));

    // Get the old room creation event
    let mut create_event_content = room::get_state_content::<CanonicalJsonObject>(
        &room_id,
        &StateEventType::RoomCreate,
        "",
        None,
    )?;
    if !version_rules.authorization.use_room_create_sender {
        create_event_content.insert(
            "creator".into(),
            json!(sender_id)
                .try_into()
                .map_err(|_| MatrixError::bad_json("error forming creation event"))?,
        );
    } else {
        // "creator" key no longer exists in V11+ rooms
        create_event_content.remove("creator");
    }
    if version_rules.authorization.additional_room_creators && !body.additional_creators.is_empty()
    {
        create_event_content.insert(
            "additional_creators".into(),
            json!(&body.additional_creators)
                .try_into()
                .map_err(|_| MatrixError::bad_json("error forming additional_creators"))?,
        );
    }

    create_event_content.insert(
        "room_version".into(),
        json!(&body.new_version)
            .try_into()
            .map_err(|_| MatrixError::bad_json("error forming creation event"))?,
    );
    create_event_content.insert(
        "predecessor".into(),
        json!(predecessor)
            .try_into()
            .map_err(|_| MatrixError::bad_json("error forming creation event"))?,
    );
    // Validate creation event content
    let de_result = serde_json::from_str::<CanonicalJsonObject>(
        to_raw_value(&create_event_content)
            .expect("error forming creation event")
            .get(),
    );

    if de_result.is_err() {
        return Err(MatrixError::bad_json("error forming creation event").into());
    }

    let new_create_event = timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomCreate,
            content: to_raw_value(&create_event_content)
                .expect("event is valid, we just created it"),
            state_key: Some("".to_owned()),
            ..Default::default()
        },
        sender_id,
        &new_room_id,
        &crate::room::get_version(&new_room_id)?,
        &state_lock,
    )
    .await?;

    // Room Version 12+ use temp room id before.
    let new_room_id = new_create_event.room_id.clone();

    let new_create_event = RoomCreateEvent::new(new_create_event.pdu);

    // Send a m.room.tombstone event to the old room to indicate that it is not intended to be used
    // any further Fail if the sender does not have the required permissions
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomTombstone,
            content: to_raw_value(&RoomTombstoneEventContent {
                body: "This room has been replaced".to_owned(),
                replacement_room: new_room_id.clone(),
            })?,
            state_key: Some("".to_owned()),
            ..Default::default()
        },
        sender_id,
        &room_id,
        &crate::room::get_version(&room_id)?,
        &state_lock,
    )
    .await?;

    // // Use the m.room.tombstone event as the predecessor
    // let predecessor = Some(crate::core::events::room::create::PreviousRoom::new(
    //     room_id.clone(),
    //     (*tombstone_event_id).to_owned(),
    // ));

    // Join the new room
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&RoomMemberEventContent {
                membership: MembershipState::Join,
                display_name: crate::data::user::display_name(sender_id).ok().flatten(),
                avatar_url: crate::data::user::avatar_url(sender_id).ok().flatten(),
                is_direct: None,
                third_party_invite: None,
                blurhash: crate::data::user::blurhash(sender_id).ok().flatten(),
                reason: None,
                join_authorized_via_users_server: None,
                extra_data: Default::default(),
            })
            .expect("event is valid, we just created it"),
            state_key: Some(sender_id.to_string()),
            ..Default::default()
        },
        sender_id,
        &new_room_id,
        &body.new_version,
        &state_lock,
    )
    .await?;

    // Recommended transferable state events list from the specs
    let transferable_state_events = vec![
        StateEventType::RoomServerAcl,
        StateEventType::RoomEncryption,
        StateEventType::RoomName,
        StateEventType::RoomAvatar,
        StateEventType::RoomTopic,
        StateEventType::RoomGuestAccess,
        StateEventType::RoomHistoryVisibility,
        StateEventType::RoomJoinRules,
        StateEventType::RoomPowerLevels,
    ];

    // Replicate transferable state events to the new room
    for event_ty in transferable_state_events {
        if event_ty == StateEventType::RoomPowerLevels {
            continue; // Handled later
        }
        let event_content = match room::get_state(&room_id, &event_ty, "", None) {
            Ok(v) => v.content.clone(),
            _ => continue, // Skipping missing events.
        };

        timeline::build_and_append_pdu(
            PduBuilder {
                event_type: event_ty.to_string().into(),
                content: event_content,
                state_key: Some("".to_owned()),
                ..Default::default()
            },
            sender_id,
            &new_room_id,
            &body.new_version,
            &state_lock,
        )
        .await?;
    }

    // Moves any local aliases to the new room
    for alias in room::local_aliases_for_room(&room_id)? {
        room::set_alias(&new_room_id, &alias, sender_id)?;
    }

    // Get the old room power levels
    let mut power_levels_event_content = room::get_state_content::<RoomPowerLevelsEventContent>(
        &room_id,
        &StateEventType::RoomPowerLevels,
        "",
        None,
    )?;

    // Setting events_default and invite to the greater of 50 and users_default + 1
    let restricted_level = max(50, power_levels_event_content.users_default + 1);
    if power_levels_event_content.events_default < restricted_level {
        power_levels_event_content.events_default = restricted_level;
    }
    if power_levels_event_content.invite < restricted_level {
        power_levels_event_content.invite = restricted_level;
    }
    // Modify the power levels in the old room to prevent sending of events and inviting new users
    let _ = timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomPowerLevels,
            content: to_raw_value(&power_levels_event_content)
                .expect("event is valid, we just created it"),
            state_key: Some("".to_owned()),
            ..Default::default()
        },
        sender_id,
        &room_id,
        &crate::room::get_version(&room_id)?,
        &state_lock,
    )
    .await?;

    if version_rules
        .authorization
        .explicitly_privilege_room_creators
    {
        let creators = new_create_event.creators()?;
        for creator in &creators {
            power_levels_event_content.users.remove(creator);
        }
        power_levels_event_content.users.remove(sender_id);
    }
    let _ = timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomPowerLevels,
            content: to_raw_value(&power_levels_event_content)
                .expect("event is valid, we just created it"),
            state_key: Some("".to_owned()),
            ..Default::default()
        },
        sender_id,
        &new_room_id,
        &body.new_version,
        &state_lock,
    )
    .await?;

    // Return the replacement room id
    json_ok(UpgradeRoomResBody {
        replacement_room: new_room_id,
    })
}

/// #GET /_matrix/client/r0/publicRooms
/// Lists the public rooms on this server.
///
/// - Rooms are ordered by the number of joined members
#[endpoint]
pub(super) async fn get_public_rooms(
    _aa: AuthArgs,
    args: PublicRoomsReqArgs,
) -> JsonResult<PublicRoomsResBody> {
    let body = crate::directory::get_public_rooms(
        args.server.as_deref(),
        args.limit,
        args.since.as_deref(),
        &PublicRoomFilter::default(),
        &RoomNetwork::Matrix,
    )
    .await?;
    json_ok(body)
}

/// #POST /_matrix/client/r0/publicRooms
/// Lists the public rooms on this server.
///
/// - Rooms are ordered by the number of joined members
#[endpoint]
pub(super) async fn get_filtered_public_rooms(
    _aa: AuthArgs,
    args: JsonBody<PublicRoomsFilteredReqBody>,
) -> JsonResult<PublicRoomsResBody> {
    let body = crate::directory::get_public_rooms(
        args.server.as_deref(),
        args.limit,
        args.since.as_deref(),
        &args.filter,
        &args.room_network,
    )
    .await?;
    json_ok(body)
}

/// #POST /_matrix/client/r0/createRoom
/// Creates a new room.
///
/// - Room ID is randomly generated
/// - Create alias if room_alias_name is set
/// - Send create event
/// - Join sender user
/// - Send power levels event
/// - Send canonical room alias
/// - Send join rules
/// - Send history visibility
/// - Send guest access
/// - Send events listed in initial state
/// - Send events implied by `name` and `topic`
/// - Send invite events
#[endpoint]
pub(super) async fn create_room(
    _aa: AuthArgs,
    body: JsonBody<CreateRoomReqBody>,
    depot: &mut Depot,
) -> JsonResult<CreateRoomResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();

    let conf = config::get();
    // let room_version =   conf.default_room_version.clone();
    let room_version = match body.room_version.clone() {
        Some(room_version) => {
            if config::supports_room_version(&room_version) {
                room_version
            } else {
                return Err(MatrixError::unsupported_room_version(
                    "this server does not support that room version",
                )
                .into());
            }
        }
        None => conf.default_room_version.clone(),
    };
    let version_rules = crate::room::get_version_rules(&room_version)?;

    if !conf.allow_room_creation && authed.appservice.is_none() && !authed.is_admin() {
        return Err(MatrixError::forbidden("room creation has been disabled", None).into());
    }

    let alias: Option<OwnedRoomAliasId> = if let Some(localpart) = &body.room_alias_name {
        // TODO: Check for invalid characters and maximum length
        let alias = RoomAliasId::parse(format!("#{}:{}", localpart, &conf.server_name))
            .map_err(|_| MatrixError::invalid_param("Invalid alias."))?;

        if room::resolve_local_alias(&alias).is_ok() {
            return Err(MatrixError::room_in_use("room alias already exists").into());
        } else {
            Some(alias)
        }
    } else {
        None
    };

    // Figure out preset. We need it for preset specific events
    let preset = body.preset.clone().unwrap_or(match &body.visibility {
        Visibility::Private => RoomPreset::PrivateChat,
        Visibility::Public => RoomPreset::PublicChat,
        _ => RoomPreset::PrivateChat, // Room visibility should not be custom
    });

    let (room_id, state_lock) = match version_rules.room_id_format {
        RoomIdFormatVersion::V1 => {
            create_create_event_legacy(sender_id, &body, &room_version, &version_rules).await?
        }
        RoomIdFormatVersion::V2 => {
            create_create_event(sender_id, &body, &preset, &room_version, &version_rules).await?
        }
    };

    // 2. Let the room creator join
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomMember,
            content: to_raw_value(&RoomMemberEventContent {
                membership: MembershipState::Join,
                display_name: crate::data::user::display_name(sender_id).ok().flatten(),
                avatar_url: crate::data::user::avatar_url(sender_id).ok().flatten(),
                is_direct: Some(body.is_direct),
                third_party_invite: None,
                blurhash: crate::data::user::blurhash(sender_id).ok().flatten(),
                reason: None,
                join_authorized_via_users_server: None,
                extra_data: Default::default(),
            })
            .expect("event is valid, we just created it"),
            state_key: Some(sender_id.to_string()),
            ..Default::default()
        },
        sender_id,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 3. Power levels
    let mut users = BTreeMap::new();
    if !version_rules
        .authorization
        .explicitly_privilege_room_creators
    {
        users.insert(sender_id.to_owned(), 100);
        if preset == RoomPreset::TrustedPrivateChat {
            for invitee_id in &body.invite {
                if user_is_ignored(sender_id, invitee_id) || user_is_ignored(invitee_id, sender_id)
                {
                    continue;
                }
                users.insert(invitee_id.to_owned(), 100);
            }
        }
    }

    let power_levels_content = default_power_levels_content(
        &version_rules.authorization,
        body.preset.as_ref(),
        body.power_level_content_override.as_ref(),
        &body.visibility,
        users,
    )?;

    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomPowerLevels,
            content: to_raw_value(&power_levels_content)?,
            state_key: Some("".to_owned()),
            ..Default::default()
        },
        sender_id,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 4. Canonical room alias
    if let Some(room_alias_id) = &alias {
        timeline::build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomCanonicalAlias,
                content: to_raw_value(&RoomCanonicalAliasEventContent {
                    alias: Some(room_alias_id.to_owned()),
                    alt_aliases: vec![],
                })
                .expect("we checked that alias earlier, it must be fine"),
                state_key: Some("".to_owned()),
                ..Default::default()
            },
            sender_id,
            &room_id,
            &room_version,
            &state_lock,
        )
        .await
        .unwrap();
    }

    // 5. Events set by preset
    // 5.1 Join Rules
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomJoinRules,
            content: to_raw_value(&RoomJoinRulesEventContent::new(match preset {
                RoomPreset::PublicChat => JoinRule::Public,
                // according to spec "invite" is the default
                _ => JoinRule::Invite,
            }))
            .expect("event is valid, we just created it"),
            state_key: Some("".to_owned()),
            ..Default::default()
        },
        sender_id,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 5.2 History Visibility
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomHistoryVisibility,
            content: to_raw_value(&RoomHistoryVisibilityEventContent::new(
                HistoryVisibility::Shared,
            ))
            .expect("event is valid, we just created it"),
            state_key: Some("".to_owned()),
            ..Default::default()
        },
        sender_id,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    // 5.3 Guest Access
    // timeline::build_and_append_pdu(
    //     PduBuilder {
    //         event_type: TimelineEventType::RoomGuestAccess,
    //         content: to_raw_value(&RoomGuestAccessEventContent::new(match preset {
    //             RoomPreset::PublicChat => GuestAccess::Forbidden,
    //             _ => GuestAccess::CanJoin,
    //         }))
    //         .expect("event is valid, we just created it"),
    //         state_key: Some("".to_owned()),
    //         ..Default::default()
    //     },
    //     sender_id,
    //     &room_id,
    //     &state_lock,
    // )?;
    if preset != RoomPreset::PublicChat {
        timeline::build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomGuestAccess,
                content: to_raw_value(&RoomGuestAccessEventContent::new(GuestAccess::CanJoin))
                    .expect("event is valid, we just created it"),
                state_key: Some("".to_owned()),
                ..Default::default()
            },
            sender_id,
            &room_id,
            &room_version,
            &state_lock,
        )
        .await?;
    }

    // 6. Events listed in initial_state
    for event in &body.initial_state {
        let mut pdu_builder = event.deserialize_as::<PduBuilder>().map_err(|e| {
            warn!("Invalid initial state event: {:?}", e);
            MatrixError::invalid_param("Invalid initial state event.")
        })?;

        // Implicit state key defaults to ""
        pdu_builder.state_key.get_or_insert_with(|| "".to_owned());

        // Silently skip encryption events if they are not allowed
        if pdu_builder.event_type == TimelineEventType::RoomEncryption && !conf.allow_encryption {
            continue;
        }

        timeline::build_and_append_pdu(
            pdu_builder,
            sender_id,
            &room_id,
            &room_version,
            &state_lock,
        )
        .await?;
    }

    // 7. Events implied by name and topic
    if let Some(name) = &body.name {
        timeline::build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomName,
                content: to_raw_value(&RoomNameEventContent::new(name.clone()))
                    .expect("event is valid, we just created it"),
                state_key: Some("".to_owned()),
                ..Default::default()
            },
            sender_id,
            &room_id,
            &room_version,
            &state_lock,
        )
        .await?;
    }

    if let Some(topic) = &body.topic {
        timeline::build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomTopic,
                content: to_raw_value(&RoomTopicEventContent::new(topic.clone()))
                    .expect("event is valid, we just created it"),
                state_key: Some("".to_owned()),
                ..Default::default()
            },
            sender_id,
            &room_id,
            &room_version,
            &state_lock,
        )
        .await?;
    }
    drop(state_lock);

    // 8. Events implied by invite (and TODO: invite_3pid)
    for user_id in &body.invite {
        if let Err(e) =
            crate::membership::invite_user(sender_id, user_id, &room_id, None, body.is_direct).await
        {
            tracing::error!("Failed to invite user {}: {:?}", user_id, e);
        }
    }

    // Homeserver specific stuff
    if let Some(alias) = alias {
        room::set_alias(&room_id, &alias, sender_id)?;
    }

    if body.visibility == Visibility::Public {
        room::directory::set_public(&room_id, true)?;
    }

    info!("{} created a room", sender_id);
    json_ok(CreateRoomResBody { room_id })
}

async fn create_create_event_legacy(
    sender_id: &UserId,
    body: &CreateRoomReqBody,
    room_version: &RoomVersionId,
    _version_rules: &RoomVersionRules,
) -> AppResult<(OwnedRoomId, RoomMutexGuard)> {
    // let room_id: OwnedRoomId = match &body.room_id {
    //     None => RoomId::new_v1(&config::get().server_name),
    //     Some(custom_id) => custom_room_id_check(custom_id).await?,
    // };
    let room_id = RoomId::new_v1(&config::get().server_name);
    let state_lock = room::lock_state(&room_id).await;
    room::ensure_room(&room_id, room_version)?;

    let content = match &body.creation_content {
        Some(content) => {
            let mut content = content
                .deserialize_as::<CanonicalJsonObject>()
                .expect("Invalid creation content");
            content.insert(
                "creator".into(),
                json!(sender_id)
                    .try_into()
                    .map_err(|_| MatrixError::bad_json("Invalid creation content"))?,
            );
            content.insert(
                "room_version".into(),
                json!(room_version.as_str())
                    .try_into()
                    .map_err(|_| MatrixError::bad_json("Invalid creation content"))?,
            );
            content
        }
        None => {
            // TODO: Add correct value for v11
            let mut content = serde_json::from_str::<CanonicalJsonObject>(
                to_raw_value(&RoomCreateEventContent::new_v1(sender_id.to_owned()))
                    .map_err(|_| MatrixError::bad_json("Invalid creation content"))?
                    .get(),
            )?;
            content.insert(
                "room_version".into(),
                json!(room_version.as_str())
                    .try_into()
                    .map_err(|_| MatrixError::bad_json("Invalid creation content"))?,
            );
            content
        }
    };

    // Validate creation content
    let de_result = serde_json::from_str::<CanonicalJsonObject>(
        to_raw_value(&content)
            .expect("Invalid creation content")
            .get(),
    );

    if de_result.is_err() {
        return Err(MatrixError::bad_json("Invalid creation content").into());
    }

    // 1. The room create event
    timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomCreate,
            content: to_raw_value(&content)?,
            state_key: Some("".to_owned()),
            ..Default::default()
        },
        sender_id,
        &room_id,
        room_version,
        &state_lock,
    )
    .await?;

    Ok((room_id, state_lock))
}

async fn create_create_event(
    sender_id: &UserId,
    body: &CreateRoomReqBody,
    preset: &RoomPreset,
    room_version: &RoomVersionId,
    version_rules: &RoomVersionRules,
) -> AppResult<(OwnedRoomId, RoomMutexGuard)> {
    let mut create_content = match &body.creation_content {
        Some(content) => {
            let mut content = content.deserialize_as::<CanonicalJsonObject>()?;

            content.insert(
                "room_version".into(),
                json!(room_version.as_str())
                    .try_into()
                    .map_err(|e| MatrixError::bad_json(format!("invalid creation content: {e}")))?,
            );

            content
        }
        None => {
            let content = RoomCreateEventContent::new_v11();

            let mut content =
                serde_json::from_str::<CanonicalJsonObject>(to_raw_value(&content)?.get())?;

            content.insert(
                "room_version".into(),
                json!(room_version.as_str()).try_into()?,
            );
            content
        }
    };

    if version_rules.authorization.additional_room_creators {
        let mut additional_creators = body
            .creation_content
            .as_ref()
            .and_then(|c| c.deserialize_as::<CreationContent>().ok())
            .unwrap_or_default()
            .additional_creators;

        if *preset == RoomPreset::TrustedPrivateChat {
            additional_creators.extend(body.invite.clone());
        }

        additional_creators.sort();
        additional_creators.dedup();
        if !additional_creators.is_empty() {
            create_content.insert(
                "additional_creators".into(),
                json!(additional_creators).try_into()?,
            );
        }
    }

    // 1. The room create event, using a placeholder room_id
    let temp_room_id = OwnedRoomId::try_from("!placehold").expect("invalid room id");
    let state_lock = room::lock_state(&temp_room_id).await;
    room::ensure_room(&temp_room_id, room_version)?;
    let create_event = timeline::build_and_append_pdu(
        PduBuilder {
            event_type: TimelineEventType::RoomCreate,
            content: to_raw_value(&create_content)?,
            state_key: Some("".to_owned()),
            ..Default::default()
        },
        sender_id,
        &temp_room_id,
        room_version,
        &state_lock,
    )
    .await?;
    drop(state_lock);

    let state_lock = room::lock_state(&create_event.room_id).await;

    Ok((create_event.room_id.clone(), state_lock))
}

// /// if a room is being created with a custom room ID, run our checks against it
// async fn custom_room_id_check(custom_room_id: &str) -> AppResult<OwnedRoomId> {
//     let conf = crate::config::get();
//     // apply forbidden room alias checks to custom room IDs too
//     if conf.forbidden_alias_names.is_match(custom_room_id) {
//         return Err(MatrixError::unknown("Custom room ID is forbidden.").into());
//     }

//     if custom_room_id.contains(':') {
//         return Err(MatrixError::invalid_param(
//             "Custom room ID contained `:` which is not allowed. Please note that this expects a \
// 			 localpart, not the full room ID.",
//         )
//         .into());
//     } else if custom_room_id.contains(char::is_whitespace) {
//         return Err(MatrixError::invalid_param(
//             "Custom room ID contained spaces which is not valid.",
//         )
//         .into());
//     }

//     let full_room_id = format!("!{custom_room_id}:{}", conf.server_name);

//     let room_id = OwnedRoomId::parse(full_room_id)
//         .inspect(|full_room_id| debug!(?full_room_id, "Full custom room ID"))
//         .inspect_err(|e| {
//             warn!(
//                 ?e,
//                 ?custom_room_id,
//                 "Failed to create room with custom room ID"
//             );
//         })?;

//     // check if room ID doesn't already exist instead of erroring on auth check
//     if room::room_exists(&room_id)? {
//         return Err(
//             MatrixError::room_in_use("Room with that custom room ID already exists").into(),
//         );
//     }

//     Ok(room_id)
// }

/// creates the power_levels_content for the PDU builder
fn default_power_levels_content(
    auth_rules: &AuthorizationRules,
    preset: Option<&RoomPreset>,
    power_level_content_override: Option<&RawJson<RoomPowerLevelsEventContent>>, /* must be raw_json */
    visibility: &Visibility,
    users: BTreeMap<OwnedUserId, i64>,
) -> AppResult<serde_json::Value> {
    let mut power_levels_content = json!(RoomPowerLevelsEventContent {
        users,
        ..RoomPowerLevelsEventContent::new(auth_rules)
    });

    // secure proper defaults of sensitive/dangerous permissions that moderators
    // (power level 50) should not have easy access to
    power_levels_content["events"]["m.room.power_levels"] = json!(100);
    power_levels_content["events"]["m.room.server_acl"] = json!(100);
    if auth_rules.explicitly_privilege_room_creators {
        power_levels_content["events"]["m.room.tombstone"] = json!(150);
    } else {
        power_levels_content["events"]["m.room.tombstone"] = json!(100);
    }

    power_levels_content["events"]["m.room.encryption"] = json!(100);
    power_levels_content["events"]["m.room.history_visibility"] = json!(100);

    // always allow users to respond (not post new) to polls. this is primarily
    // useful in read-only announcement rooms that post a public poll.
    power_levels_content["events"]["org.matrix.msc3381.poll.response"] = json!(0);
    power_levels_content["events"]["m.poll.response"] = json!(0);

    // synapse does this too. clients do not expose these permissions. it prevents
    // default users from calling public rooms, for obvious reasons.
    if *visibility == Visibility::Public {
        power_levels_content["events"]["m.call.invite"] = json!(50);
        power_levels_content["events"]["m.call"] = json!(50);
        power_levels_content["events"]["m.call.member"] = json!(50);
        power_levels_content["events"]["org.matrix.msc3401.call"] = json!(50);
        power_levels_content["events"]["org.matrix.msc3401.call.member"] = json!(50);
    }

    if preset == Some(&RoomPreset::TrustedPrivateChat) || preset == Some(&RoomPreset::PrivateChat) {
        power_levels_content["invite"] = json!(0);
    }

    if let Some(power_level_content_override) = power_level_content_override {
        let JsonValue::Object(json) =
            serde_json::from_str(power_level_content_override.inner().get())
                .map_err(|_| MatrixError::bad_json("invalid power_level_content_override"))?
        else {
            return Err(MatrixError::bad_json("invalid power_level_content_override").into());
        };

        for (key, value) in json {
            power_levels_content[key] = value;
        }
    }

    Ok(power_levels_content)
}
