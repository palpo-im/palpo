use std::collections::BTreeMap;

use diesel::prelude::*;
use palpo_core::UnixMillis;
use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{from_value as from_json_value, value::to_raw_value};

use crate::core::client::profile::*;
use crate::core::events::room::member::RoomMemberEventContent;
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::federation::query::{
    ProfileReqArgs, ProfileResBody as FederationProfileResBody, profile_request,
};
use crate::core::identifiers::*;
use crate::core::profile::ProfileFieldName;
use crate::core::serde::{JsonObject, JsonValue};
use crate::core::user::ProfileResBody;
use crate::data::schema::*;
use crate::data::user::{DbProfile, NewDbPresence};
use crate::data::{connect, diesel_exists};
use crate::exts::*;
use crate::room::timeline;
use crate::{
    AppError, AuthArgs, EmptyResult, JsonResult, MatrixError, PduBuilder, data, empty_ok, hoops,
    json_ok, room,
};

pub fn public_router() -> Router {
    Router::with_path("profile/{user_id}")
        .get(get_profile)
        .push(Router::with_path("avatar_url").get(get_avatar_url))
        .push(Router::with_path("displayname").get(get_display_name))
        .push(Router::with_path("{field}").get(get_profile_field))
}
pub fn authed_router() -> Router {
    Router::with_path("profile/{user_id}")
        .hoop(hoops::limit_rate)
        .push(Router::with_path("avatar_url").put(set_avatar_url))
        .push(Router::with_path("displayname").put(set_display_name))
        .push(
            Router::with_path("{field}")
                .put(set_profile_field)
                .delete(delete_profile_field),
        )
}

#[derive(ToSchema, Serialize, Deserialize, Debug, Default)]
struct ProfileFieldBody {
    #[serde(flatten)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    fields: JsonObject,
}

impl ProfileFieldBody {
    fn single(field: impl Into<String>, value: JsonValue) -> Self {
        let mut fields = JsonObject::new();
        fields.insert(field.into(), value);
        Self { fields }
    }
}

fn profile_from_federation_response(
    profile: FederationProfileResBody,
) -> serde_json::Result<ProfileResBody> {
    let fields = profile
        .iter()
        .filter(|(field, _)| {
            !matches!(
                field.as_str(),
                "avatar_url" | "displayname" | "xyz.amorgan.blurhash"
            )
        })
        .map(|(field, value)| (field.clone(), value.clone()))
        .collect();

    Ok(ProfileResBody {
        avatar_url: profile
            .get("avatar_url")
            .cloned()
            .map(from_json_value)
            .transpose()?,
        display_name: profile
            .get("displayname")
            .cloned()
            .map(from_json_value)
            .transpose()?,
        blurhash: profile
            .get("xyz.amorgan.blurhash")
            .and_then(|value| value.as_str().map(ToOwned::to_owned)),
        fields,
    })
}

fn custom_profile_fields(fields: JsonValue) -> BTreeMap<String, JsonValue> {
    fields
        .as_object()
        .map(|fields| {
            fields
                .iter()
                .map(|(field, value)| (field.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn ensure_custom_profile_field(field: &str) -> Result<(), MatrixError> {
    if matches!(field, "avatar_url" | "displayname" | "xyz.amorgan.blurhash") {
        return Err(MatrixError::invalid_param(
            "Use the dedicated profile endpoint for this field.",
        ));
    }

    Ok(())
}

fn ensure_profile_update_allowed(
    authed: &crate::AuthedInfo,
    user_id: &UserId,
) -> Result<(), MatrixError> {
    let is_allowed = authed.user_id() == user_id
        || authed
            .appservice()
            .is_some_and(|appservice| appservice.is_user_match(user_id));

    if !is_allowed {
        return Err(MatrixError::forbidden("forbidden", None));
    }

    Ok(())
}

/// #GET /_matrix/client/r0/profile/{user_id}
/// Returns the display_name, avatar_url and blurhash of the user.
///
/// - If user is on another server: Fetches profile over federation
#[endpoint]
async fn get_profile(_aa: AuthArgs, user_id: PathParam<OwnedUserId>) -> JsonResult<ProfileResBody> {
    let user_id = user_id.into_inner();
    let server_name = user_id.server_name().to_owned();
    if !server_name.is_valid() {
        return Err(MatrixError::not_found("profile not found").into());
    }
    if user_id.is_remote() {
        let request = profile_request(
            &server_name.origin().await,
            ProfileReqArgs {
                user_id,
                field: None,
            },
        )?
        .into_inner();

        let profile = crate::sending::send_federation_request(&server_name, request, Some(5))
            .await?
            .json::<FederationProfileResBody>()
            .await?;
        return json_ok(profile_from_federation_response(profile)?);
    }
    let Ok(DbProfile {
        blurhash,
        avatar_url,
        display_name,
        fields,
        ..
    }) = user_profiles::table
        .filter(user_profiles::user_id.eq(&user_id))
        .filter(user_profiles::room_id.is_null())
        .first::<DbProfile>(&mut connect()?)
    else {
        return json_ok(ProfileResBody {
            avatar_url: None,
            blurhash: None,
            display_name: Some(user_id.localpart().to_owned()),
            fields: BTreeMap::new(),
        });
    };

    json_ok(ProfileResBody {
        avatar_url,
        blurhash,
        display_name,
        fields: custom_profile_fields(fields),
    })
}

/// #GET /_matrix/client/v3/profile/{user_id}/{field}
/// Returns one custom profile field.
#[endpoint]
async fn get_profile_field(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    field: PathParam<String>,
) -> JsonResult<ProfileFieldBody> {
    let user_id = user_id.into_inner();
    let field = field.into_inner();
    ensure_custom_profile_field(&field)?;

    if user_id.is_remote() {
        let server_name = user_id.server_name().to_owned();
        let request = profile_request(
            &server_name.origin().await,
            ProfileReqArgs {
                user_id,
                field: Some(field.as_str().into()),
            },
        )?
        .into_inner();

        let profile = crate::sending::send_federation_request(&server_name, request, None)
            .await?
            .json::<FederationProfileResBody>()
            .await?;

        let value = profile
            .get(&field)
            .cloned()
            .ok_or_else(|| MatrixError::not_found("Profile field not found."))?;

        return json_ok(ProfileFieldBody::single(field, value));
    }

    let value = data::user::profile_field(&user_id, &field)?
        .ok_or_else(|| MatrixError::not_found("Profile field not found."))?;

    json_ok(ProfileFieldBody::single(field, value))
}

/// #GET /_matrix/client/r0/profile/{user_id}/avatar_url
/// Returns the avatar_url and blurhash of the user.
///
/// - If user is on another server: Fetches avatar_url and blurhash over federation
#[endpoint]
async fn get_avatar_url(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
) -> JsonResult<AvatarUrlResBody> {
    let user_id = user_id.into_inner();
    if user_id.is_remote() {
        let server_name = user_id.server_name().to_owned();
        let request = profile_request(
            &server_name.origin().await,
            ProfileReqArgs {
                user_id,
                field: Some(ProfileFieldName::AvatarUrl),
            },
        )?
        .into_inner();

        let body: AvatarUrlResBody =
            crate::sending::send_federation_request(&server_name, request, None)
                .await?
                .json::<AvatarUrlResBody>()
                .await?;
        return json_ok(body);
    }

    let DbProfile {
        avatar_url,
        blurhash,
        ..
    } = user_profiles::table
        .filter(user_profiles::user_id.eq(&user_id))
        .first::<DbProfile>(&mut connect()?)?;

    json_ok(AvatarUrlResBody {
        avatar_url,
        blurhash,
    })
}

/// #PUT /_matrix/client/r0/profile/{user_id}/avatar_url
/// Updates the avatar_url and blurhash.
///
/// - Also makes sure other users receive the update using presence EDUs
#[endpoint]
async fn set_avatar_url(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    body: JsonBody<SetAvatarUrlReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let user_id = user_id.into_inner();
    let authed = depot.authed_info()?;

    // Allow if the user is updating their own profile, or if an appservice is updating
    // a user within its namespace
    ensure_profile_update_allowed(authed, &user_id)?;

    let SetAvatarUrlReqBody {
        avatar_url,
        blurhash,
    } = body.into_inner();

    let query = user_profiles::table
        .filter(user_profiles::user_id.eq(&user_id))
        .filter(user_profiles::room_id.is_null());
    let profile_exists = diesel_exists!(query, &mut connect()?)?;
    if profile_exists {
        #[derive(AsChangeset, Debug)]
        #[diesel(table_name = user_profiles, treat_none_as_null = true)]
        struct UpdateParams {
            avatar_url: Option<OwnedMxcUri>,
            blurhash: Option<String>,
        }
        let updata_params = UpdateParams {
            avatar_url: avatar_url.clone(),
            blurhash,
        };
        diesel::update(query)
            .set(updata_params)
            .execute(&mut connect()?)?;
    } else {
        return Err(StatusError::not_found().brief("Profile not found.").into());
    }

    // Send a new membership event and presence update into all joined rooms
    let all_joined_rooms: Vec<_> = data::user::joined_rooms(&user_id)?
        .into_iter()
        .map(|room_id| {
            Ok::<_, AppError>((
                PduBuilder {
                    event_type: TimelineEventType::RoomMember,
                    content: to_raw_value(&RoomMemberEventContent {
                        avatar_url: avatar_url.clone(),
                        ..room::get_state_content::<RoomMemberEventContent>(
                            &room_id,
                            &StateEventType::RoomMember,
                            user_id.as_str(),
                            None,
                        )?
                    })
                    .expect("event is valid, we just created it"),
                    state_key: Some(user_id.to_string()),
                    ..Default::default()
                },
                room_id,
            ))
        })
        .filter_map(|r| r.ok())
        .collect();

    // Presence update
    crate::data::user::set_presence(
        NewDbPresence {
            user_id: user_id.clone(),
            stream_id: None,
            state: None,
            status_msg: None,
            last_active_at: Some(UnixMillis::now()),
            last_federation_update_at: None,
            last_user_sync_at: None,
            currently_active: None,
            occur_sn: None,
        },
        true,
    )?;
    for (pdu_builder, room_id) in all_joined_rooms {
        let _ = timeline::build_and_append_pdu(
            pdu_builder,
            &user_id,
            &room_id,
            &room::get_version(&room_id)?,
            &room::lock_state(&room_id).await,
        )
        .await?;
    }

    empty_ok()
}

/// #GET /_matrix/client/r0/profile/{user_id}/displayname
/// Returns the display_name of the user.
///
/// - If user is on another server: Fetches display_name over federation
#[endpoint]
async fn get_display_name(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
) -> JsonResult<DisplayNameResBody> {
    let user_id = user_id.into_inner();
    if user_id.is_remote() {
        let server_name = user_id.server_name().to_owned();
        let request = profile_request(
            &server_name.origin().await,
            ProfileReqArgs {
                user_id,
                field: Some(ProfileFieldName::DisplayName),
            },
        )?
        .into_inner();

        let body = crate::sending::send_federation_request(&server_name, request, None)
            .await?
            .json::<DisplayNameResBody>()
            .await?;
        return json_ok(body);
    }
    json_ok(DisplayNameResBody {
        display_name: data::user::display_name(&user_id).ok().flatten(),
    })
}

/// #PUT /_matrix/client/r0/profile/{user_id}/displayname
/// Updates the display_name.
///
/// - Also makes sure other users receive the update using presence EDUs
#[endpoint]
async fn set_display_name(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    body: JsonBody<SetDisplayNameReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let user_id = user_id.into_inner();
    let authed = depot.authed_info()?;

    // Allow if the user is updating their own profile, or if an appservice is updating
    // a user within its namespace
    ensure_profile_update_allowed(authed, &user_id)?;
    let SetDisplayNameReqBody { display_name } = body.into_inner();

    if let Some(display_name) = display_name.as_deref() {
        data::user::set_display_name(&user_id, display_name)?;
    }

    // Send a new membership event and presence update into all joined rooms
    let all_joined_rooms: Vec<_> = data::user::joined_rooms(&user_id)?
        .into_iter()
        .map(|room_id| {
            Ok::<_, AppError>((
                PduBuilder {
                    event_type: TimelineEventType::RoomMember,
                    content: to_raw_value(&RoomMemberEventContent {
                        display_name: display_name.clone(),
                        ..room::get_state_content::<RoomMemberEventContent>(
                            &room_id,
                            &StateEventType::RoomMember,
                            user_id.as_str(),
                            None,
                        )?
                    })
                    .expect("event is valid, we just created it"),
                    state_key: Some(user_id.to_string()),
                    ..Default::default()
                },
                room_id,
            ))
        })
        .filter_map(|r| r.ok())
        .collect();

    for (pdu_builder, room_id) in all_joined_rooms {
        let _ = timeline::build_and_append_pdu(
            pdu_builder,
            &user_id,
            &room_id,
            &crate::room::get_version(&room_id)?,
            &room::lock_state(&room_id).await,
        )
        .await?;

        // Presence update
        crate::data::user::set_presence(
            NewDbPresence {
                user_id: user_id.clone(),
                stream_id: None,
                state: None,
                status_msg: None,
                last_active_at: Some(UnixMillis::now()),
                last_federation_update_at: None,
                last_user_sync_at: None,
                currently_active: None,
                occur_sn: None,
            },
            true,
        )?;
    }

    empty_ok()
}

/// #PUT /_matrix/client/v3/profile/{user_id}/{field}
/// Updates one custom profile field.
#[endpoint]
async fn set_profile_field(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    field: PathParam<String>,
    body: JsonBody<ProfileFieldBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let user_id = user_id.into_inner();
    let field = field.into_inner();
    ensure_custom_profile_field(&field)?;

    let authed = depot.authed_info()?;
    ensure_profile_update_allowed(authed, &user_id)?;

    let mut body = body.into_inner();
    let value = body
        .fields
        .remove(&field)
        .ok_or_else(|| MatrixError::bad_json("Profile field body does not match path field."))?;

    data::user::set_profile_field(&user_id, &field, value)?;

    empty_ok()
}

/// #DELETE /_matrix/client/v3/profile/{user_id}/{field}
/// Deletes one custom profile field.
#[endpoint]
async fn delete_profile_field(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    field: PathParam<String>,
    depot: &mut Depot,
) -> EmptyResult {
    let user_id = user_id.into_inner();
    let field = field.into_inner();
    ensure_custom_profile_field(&field)?;

    let authed = depot.authed_info()?;
    ensure_profile_update_allowed(authed, &user_id)?;

    data::user::delete_profile_field(&user_id, &field)?;

    empty_ok()
}
