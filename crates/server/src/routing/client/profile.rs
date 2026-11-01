use std::collections::BTreeMap;

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
use crate::data::user::DbProfile;
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
    if field.len() > 255 {
        return Err(MatrixError::key_too_large(
            "Profile keys must not exceed 255 bytes.",
        ));
    }
    if matches!(field, "avatar_url" | "displayname" | "xyz.amorgan.blurhash") {
        return Err(MatrixError::invalid_param(
            "Use the dedicated profile endpoint for this field.",
        ));
    }

    // Matrix Common Namespaced Identifier Grammar (v1.16 appendices).
    if !field.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
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
    // Profile values are stored and propagated without fetching them. The
    // profile API permits servers to preserve invalid values for compatibility;
    // media endpoints still validate identifiers before accessing any content.
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

    data::user::set_global_avatar_and_blurhash(user_id, avatar_url.as_deref(), blurhash.as_deref())
        .await?;

    // Send a new membership event and presence update into all joined rooms
    let mut all_joined_rooms: Vec<_> = Vec::new();
    for room_id in data::user::joined_rooms(user_id).await?.into_iter() {
        let result: Result<_, AppError> = async {
            Ok((
                PduBuilder {
                    event_type: TimelineEventType::RoomMember,
                    content: to_raw_value(&RoomMemberEventContent {
                        avatar_url: avatar_url.clone(),
                        blurhash: blurhash.clone(),
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

    #[test]
    fn oversized_profile_keys_use_the_specified_error() {
        let error = ensure_custom_profile_field(&"a".repeat(256)).unwrap_err();
        assert!(matches!(
            error.kind,
            crate::core::error::ErrorKind::KeyTooLarge
        ));
        for code in ["M_KEY_TOO_LARGE", "M_PROFILE_TOO_LARGE"] {
            let kind: crate::core::error::ErrorKind =
                serde_json::from_value(serde_json::json!({"errcode": code})).unwrap();
            assert_eq!(serde_json::to_value(kind).unwrap()["errcode"], code);
        }
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_profile_size_rejections_roll_back_fields_and_stream() {
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;
        use serde_json::json;

        use crate::data::schema::user_profile_changes;
        use crate::data::{self};

        crate::test_database::init();
        let user_id: crate::core::OwnedUserId = "@profile:example.org".try_into().unwrap();
        data::user::create_user(&data::user::NewDbUser {
            id: user_id.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: user_id.localpart().into(),
            server_name: user_id.server_name().to_owned(),
            appservice_id: None,
            created_at: crate::core::UnixMillis::now(),
        })
        .await
        .unwrap();
        data::user::ensure_profile_exists(&user_id).await.unwrap();
        // Serialized writers must validate the combined profile inside their transactions.
        let (first, second) = tokio::join!(
            data::user::set_profile_field(&user_id, "org.example.one", json!("a".repeat(35_000))),
            data::user::set_profile_field(&user_id, "org.example.two", json!("b".repeat(35_000))),
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let rejected = if first.is_err() { first } else { second };
        assert!(matches!(
            rejected,
            Err(data::DataError::Matrix(crate::core::MatrixError {
                kind: crate::core::error::ErrorKind::ProfileTooLarge,
                ..
            }))
        ));
        let before = data::user::get_profile(&user_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(before.fields.as_object().unwrap().len(), 1);
        assert!(
            data::user::set_global_display_name(&user_id, Some(&"x".repeat(35_000)))
                .await
                .is_err()
        );
        assert!(
            data::user::set_global_avatar_and_blurhash(&user_id, None, Some(&"x".repeat(35_000)))
                .await
                .is_err()
        );
        assert!(
            data::user::set_profile_field(&user_id, &"a".repeat(256), json!(1))
                .await
                .is_err()
        );
        let after = data::user::get_profile(&user_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.fields, before.fields);
        assert_eq!(after.display_name, before.display_name);
        assert_eq!(after.blurhash, before.blurhash);
        let count = user_profile_changes::table
            .filter(user_profile_changes::user_id.eq(&user_id))
            .count()
            .get_result::<i64>(&mut data::connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            count, 1,
            "rejected mutations must publish no change-stream entries"
        );
    }
}

#[cfg(test)]
mod avatar_http_tests {
    use salvo::prelude::*;
    use salvo::test::{RequestBuilder, ResponseExt, TestClient};
    use serde_json::{Value, json};

    use crate::core::{OwnedDeviceId, UnixMillis, UserId};
    use crate::{config, data};

    async fn send(request: RequestBuilder, service: &Service) -> Response {
        let router = service.router();
        tokio::spawn(async move { request.send(router).await })
            .await
            .unwrap()
    }

    async fn ok(mut response: Response) -> Value {
        let body = response.take_json::<Value>().await.unwrap();
        assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
        body
    }

    #[test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
    fn profile_http_avatar_updates_preserve_opaque_values_and_ownership() {
        std::thread::Builder::new()
            .name("profile-avatar-http".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(avatar_updates());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn avatar_updates() {
        crate::test_database::init();
        config::CONFIG
            .set(
                serde_json::from_value(json!({
                    "server_name": "profile.example", "db": {"url": "unused-test-config"},
                    "ip_range_denylist": []
                }))
                .unwrap(),
            )
            .unwrap();
        for name in ["alice", "bob"] {
            let user_id = UserId::parse(format!("@{name}:profile.example")).unwrap();
            data::user::create_user(&data::user::NewDbUser {
                id: user_id.clone(),
                ty: None,
                is_admin: false,
                is_guest: false,
                is_local: true,
                localpart: name.into(),
                server_name: user_id.server_name().to_owned(),
                appservice_id: None,
                created_at: UnixMillis::now(),
            })
            .await
            .unwrap();
            data::user::device::create_device(
                &user_id,
                &OwnedDeviceId::from("D"),
                &format!("{name}-profile-token"),
                None,
                None,
            )
            .await
            .unwrap();
        }
        let service = Service::new(crate::routing::root());
        let created = ok(send(
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .add_header("Authorization", "Bearer alice-profile-token", true)
                .json(&json!({"room_version": "11", "preset": "public_chat"})),
            &service,
        )
        .await)
        .await;
        let room_id = created["room_id"].as_str().unwrap();
        let initial = ok(send(
            TestClient::get("http://localhost/_matrix/client/v3/sync?timeout=0").add_header(
                "Authorization",
                "Bearer alice-profile-token",
                true,
            ),
            &service,
        )
        .await)
        .await;
        let since = initial["next_batch"].as_str().unwrap();
        let avatar_url =
            "http://localhost/_matrix/client/v3/profile/@alice:profile.example/avatar_url";

        for token in [None, Some("bob-profile-token")] {
            let mut request = TestClient::put(avatar_url).json(&json!({"avatar_url": "intruder"}));
            if let Some(token) = token {
                request = request.add_header("Authorization", format!("Bearer {token}"), true);
            }
            let response = send(request, &service).await;
            assert_eq!(
                response.status_code,
                Some(if token.is_some() {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::UNAUTHORIZED
                })
            );
        }

        // This is the unchanged Complement TestAvatarUrlUpdate payload.
        ok(send(
            TestClient::put(avatar_url)
                .add_header("Authorization", "Bearer alice-profile-token", true)
                .json(&json!({"avatar_url": "LemurLover"})),
            &service,
        )
        .await)
        .await;
        assert_eq!(
            ok(send(TestClient::get(avatar_url), &service).await).await["avatar_url"],
            "LemurLover"
        );
        let membership = ok(send(TestClient::get(format!(
            "http://localhost/_matrix/client/v3/rooms/{room_id}/state/m.room.member/@alice:profile.example"
        )).add_header("Authorization", "Bearer alice-profile-token", true), &service).await).await;
        assert_eq!(membership["membership"], "join");
        assert_eq!(membership["avatar_url"], "LemurLover");
        let sync = ok(send(
            TestClient::get(format!(
                "http://localhost/_matrix/client/v3/sync?timeout=0&since={since}"
            ))
            .add_header("Authorization", "Bearer alice-profile-token", true),
            &service,
        )
        .await)
        .await;
        assert!(
            sync["rooms"]["join"][room_id]["timeline"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| {
                    event["type"] == "m.room.member"
                        && event["state_key"] == "@alice:profile.example"
                        && event["content"]["avatar_url"] == "LemurLover"
                }),
            "profile update must reach the incremental sync timeline: {sync}"
        );
        let response = send(
            TestClient::put(avatar_url)
                .add_header("Authorization", "Bearer bob-profile-token", true)
                .json(&json!({"avatar_url": "overwrite"})),
            &service,
        )
        .await;
        assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
        assert_eq!(
            ok(send(TestClient::get(avatar_url), &service).await).await["avatar_url"],
            "LemurLover"
        );
    }
}
