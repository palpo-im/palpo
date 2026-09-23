use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::from_value as from_json_value;
use serde_json::value::to_raw_value;

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
use crate::data::user::DbProfile;
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
        .push(
            Router::with_path("avatar_url")
                .put(set_avatar_url)
                .delete(delete_avatar_url),
        )
        .push(
            Router::with_path("displayname")
                .put(set_display_name)
                .delete(delete_display_name),
        )
        .push(
            Router::with_path("{field}")
                .put(set_profile_field)
                .delete(delete_profile_field),
        )
}

/// MSC4133 unstable-prefixed profile reads.
///
/// Clients built on matrix-js-sdk only use the stable `/v3` profile-field
/// endpoints once the server advertises `uk.tcpip.msc4133.stable`; otherwise
/// they fall back to `/_matrix/client/unstable/uk.tcpip.msc4133/profile/...`.
/// Serving both keeps older clients working.
pub(super) fn msc4133_public_router() -> Router {
    Router::with_path("uk.tcpip.msc4133/profile/{user_id}")
        .get(get_profile)
        .push(Router::with_path("{field}").get(get_profile_field))
}

/// MSC4133 unstable-prefixed profile writes.
///
/// Pushed into an already authenticated and rate-limited router, so no hoops
/// are attached here.
pub(super) fn msc4133_authed_router() -> Router {
    Router::with_path("uk.tcpip.msc4133/profile/{user_id}/{field}")
        .put(set_profile_field)
        .delete(delete_profile_field)
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

    // Matrix Common Namespaced Identifier Grammar (v1.16 appendices).
    if field.len() > 255
        || !field.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !field.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_' | b'.')
        })
    {
        return Err(MatrixError::invalid_param("Invalid profile field name."));
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
    let profile = data::user::get_profile(&user_id, None)
        .await?
        .ok_or_else(|| MatrixError::not_found("Profile not found."))?;
    let DbProfile {
        blurhash,
        avatar_url,
        display_name,
        fields,
        ..
    } = profile;

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

    let value = data::user::profile_field(&user_id, &field)
        .await?
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
    } = data::user::get_profile(&user_id, None)
        .await?
        .ok_or_else(|| MatrixError::not_found("Avatar URL not found."))?;
    let avatar_url = avatar_url.ok_or_else(|| MatrixError::not_found("Avatar URL not found."))?;

    json_ok(AvatarUrlResBody {
        avatar_url: Some(avatar_url),
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
    if !avatar_url.is_valid() {
        return Err(MatrixError::invalid_param("Avatar URL must be an MXC URI.").into());
    }
    update_avatar_url(&user_id, Some(avatar_url), blurhash).await
}

#[endpoint]
async fn delete_avatar_url(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    depot: &mut Depot,
) -> EmptyResult {
    let user_id = user_id.into_inner();
    ensure_profile_update_allowed(depot.authed_info()?, &user_id)?;
    update_avatar_url(&user_id, None, None).await
}

async fn update_avatar_url(
    user_id: &UserId,
    avatar_url: Option<OwnedMxcUri>,
    blurhash: Option<String>,
) -> EmptyResult {
    data::user::ensure_profile_exists(user_id).await?;

    let query = user_profiles::table
        .filter(user_profiles::user_id.eq(user_id))
        .filter(user_profiles::room_id.is_null());
    let profile_exists = diesel_exists!(query, &mut connect().await?)?;
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
            .execute(&mut connect().await?)
            .await?;
    } else {
        return Err(StatusError::not_found().brief("Profile not found.").into());
    }

    // Send a new membership event and presence update into all joined rooms
    let mut all_joined_rooms: Vec<_> = Vec::new();
    for room_id in data::user::joined_rooms(user_id).await?.into_iter() {
        let result: Result<_, AppError> = async {
            Ok((
                PduBuilder {
                    event_type: TimelineEventType::RoomMember,
                    content: to_raw_value(&RoomMemberEventContent {
                        avatar_url: avatar_url.clone(),
                        ..room::get_state_content::<RoomMemberEventContent>(
                            &room_id,
                            &StateEventType::RoomMember,
                            user_id.as_str(),
                            None,
                        )
                        .await?
                    })
                    .expect("event is valid, we just created it"),
                    state_key: Some(user_id.to_string()),
                    ..Default::default()
                },
                room_id,
            ))
        }
        .await;
        if let Ok(item) = result {
            all_joined_rooms.push(item);
        }
    }

    crate::data::user::refresh_presence_for_profile(user_id).await?;
    for (pdu_builder, room_id) in all_joined_rooms {
        let _ = timeline::build_and_append_pdu(
            pdu_builder,
            user_id,
            &room_id,
            &room::get_version(&room_id).await?,
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
    let display_name = data::user::display_name(&user_id)
        .await?
        .ok_or_else(|| MatrixError::not_found("Display name not found."))?;
    json_ok(DisplayNameResBody {
        display_name: Some(display_name),
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

    let display_name =
        display_name.ok_or_else(|| MatrixError::bad_json("Display name must be a string."))?;
    update_display_name(&user_id, Some(display_name)).await
}

#[endpoint]
async fn delete_display_name(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    depot: &mut Depot,
) -> EmptyResult {
    let user_id = user_id.into_inner();
    ensure_profile_update_allowed(depot.authed_info()?, &user_id)?;
    update_display_name(&user_id, None).await
}

async fn update_display_name(user_id: &UserId, display_name: Option<String>) -> EmptyResult {
    if let Some(display_name) = display_name.as_deref() {
        data::user::set_display_name(user_id, display_name).await?;
    } else {
        data::user::remove_display_name(user_id).await?;
    }

    // Send a new membership event and presence update into all joined rooms
    let mut all_joined_rooms: Vec<_> = Vec::new();
    for room_id in data::user::joined_rooms(user_id).await?.into_iter() {
        let result: Result<_, AppError> = async {
            Ok((
                PduBuilder {
                    event_type: TimelineEventType::RoomMember,
                    content: to_raw_value(&RoomMemberEventContent {
                        display_name: display_name.clone(),
                        ..room::get_state_content::<RoomMemberEventContent>(
                            &room_id,
                            &StateEventType::RoomMember,
                            user_id.as_str(),
                            None,
                        )
                        .await?
                    })
                    .expect("event is valid, we just created it"),
                    state_key: Some(user_id.to_string()),
                    ..Default::default()
                },
                room_id,
            ))
        }
        .await;
        if let Ok(item) = result {
            all_joined_rooms.push(item);
        }
    }

    for (pdu_builder, room_id) in all_joined_rooms {
        let _ = timeline::build_and_append_pdu(
            pdu_builder,
            user_id,
            &room_id,
            &crate::room::get_version(&room_id).await?,
            &room::lock_state(&room_id).await,
        )
        .await?;
    }

    crate::data::user::refresh_presence_for_profile(user_id).await?;

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
    if body.fields.len() != 1 {
        return Err(
            MatrixError::bad_json("Profile field body must contain exactly one key.").into(),
        );
    }
    let value = body
        .fields
        .remove(&field)
        .ok_or_else(|| MatrixError::bad_json("Profile field body does not match path field."))?;

    data::user::set_profile_field(&user_id, &field, value).await?;

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

    data::user::delete_profile_field(&user_id, &field).await?;

    empty_ok()
}

#[cfg(test)]
mod tests {
    use super::ensure_custom_profile_field;

    #[test]
    fn custom_profile_keys_follow_matrix_identifier_grammar() {
        assert!(ensure_custom_profile_field("com.example.banner").is_ok());
        assert!(ensure_custom_profile_field("m.tz").is_ok());
        for field in ["", "DisplayName", "com.example/banner", "avatar_url"] {
            assert!(ensure_custom_profile_field(field).is_err(), "{field}");
        }
        assert!(ensure_custom_profile_field(&"a".repeat(256)).is_err());
    }
}
