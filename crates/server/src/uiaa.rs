use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use subtle::ConstantTimeEq;

use crate::core::client::uiaa::{
    AuthData, AuthError, AuthType, Password, UiaaInfo, UserIdentifier,
};
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonValue, JsonValue};
use crate::data::connect;
use crate::data::schema::*;
use crate::{AppResult, MatrixError, SESSION_ID_LENGTH, data, utils};

/// Creates a new Uiaa session. Make sure the session token is unique.
pub async fn create_session(
    user_id: &UserId,
    device_id: &DeviceId,
    uiaa_info: &UiaaInfo,
    json_body: CanonicalJsonValue,
) -> AppResult<()> {
    let session = uiaa_info.session.as_ref().expect("session should be set");
    // First create/update the DB row with uiaa_info
    update_session(user_id, device_id, session, Some(uiaa_info)).await?;
    // Then store the request body (now the row exists)
    set_uiaa_request(user_id, device_id, session, json_body).await?;
    Ok(())
}

pub async fn update_session(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
    uiaa_info: Option<&UiaaInfo>,
) -> AppResult<()> {
    if let Some(uiaa_info) = uiaa_info {
        let uiaa_info = serde_json::to_value(uiaa_info)?;
        diesel::insert_into(user_uiaa_datas::table)
            .values((
                user_uiaa_datas::user_id.eq(user_id),
                user_uiaa_datas::device_id.eq(device_id),
                user_uiaa_datas::session.eq(session),
                user_uiaa_datas::uiaa_info.eq(&uiaa_info),
            ))
            .on_conflict((
                user_uiaa_datas::user_id,
                user_uiaa_datas::device_id,
                user_uiaa_datas::session,
            ))
            .do_update()
            .set(user_uiaa_datas::uiaa_info.eq(&uiaa_info))
            .execute(&mut connect().await?)
            .await?;
    } else {
        diesel::delete(
            user_uiaa_datas::table
                .filter(user_uiaa_datas::user_id.eq(user_id))
                .filter(user_uiaa_datas::device_id.eq(device_id))
                .filter(user_uiaa_datas::session.eq(session)),
        )
        .execute(&mut connect().await?)
        .await?;
    };
    Ok(())
}
pub async fn get_session(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
) -> AppResult<UiaaInfo> {
    let uiaa_info = user_uiaa_datas::table
        .filter(user_uiaa_datas::user_id.eq(user_id))
        .filter(user_uiaa_datas::device_id.eq(device_id))
        .filter(user_uiaa_datas::session.eq(session))
        .select(user_uiaa_datas::uiaa_info)
        .first::<JsonValue>(&mut connect().await?)
        .await?;
    Ok(serde_json::from_value(uiaa_info)?)
}
pub async fn try_auth(
    user_id: &UserId,
    device_id: &DeviceId,
    auth: &AuthData,
    uiaa_info: &UiaaInfo,
) -> AppResult<(bool, UiaaInfo)> {
    let mut uiaa_info = match auth.session() {
        Some(session) => get_session(user_id, device_id, session).await?,
        None => uiaa_info.clone(),
    };

    if uiaa_info.session.is_none() {
        uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
    }
    let conf = crate::config::get();

    match auth {
        // Find out what the user completed
        AuthData::Password(Password {
            identifier,
            password,
            ..
        }) => {
            let username = match identifier {
                UserIdentifier::Matrix(identifier) => &identifier.user,
                _ => {
                    return Err(MatrixError::unauthorized("identifier type not recognized.").into());
                }
            };

            let auth_user_id = UserId::parse_with_server_name(username.clone(), &conf.server_name)
                .map_err(|_| MatrixError::unauthorized("User ID is invalid."))?;
            if user_id != auth_user_id {
                return Err(MatrixError::forbidden("User ID does not match.", None).into());
            }

            let Ok(user) = data::user::get_user(&auth_user_id).await else {
                return Err(MatrixError::unauthorized("user not found.").into());
            };
            crate::user::verify_password(&user, password).await?;
        }
        AuthData::RegistrationToken(t) => {
            let token_valid = conf
                .registration_token
                .as_deref()
                .map(|expected| {
                    let input = t.token.trim().as_bytes();
                    let expected = expected.as_bytes();
                    // Constant-length comparison to prevent timing attacks.
                    // Only compare if lengths match to avoid leaking length info
                    // beyond a simple equal/not-equal distinction.
                    input.len() == expected.len() && input.ct_eq(expected).into()
                })
                .unwrap_or(false);

            if token_valid {
                uiaa_info.completed.push(AuthType::RegistrationToken);
            } else {
                uiaa_info.auth_error = Some(AuthError::forbidden("Invalid registration token."));
                return Ok((false, uiaa_info));
            }
        }
        AuthData::Dummy(_) => {
            uiaa_info.completed.push(AuthType::Dummy);
        }
        k => error!("type not supported: {:?}", k),
    }

    // Check if a flow now succeeds
    let mut completed = false;
    'flows: for flow in &mut uiaa_info.flows {
        for stage in &flow.stages {
            if !uiaa_info.completed.contains(stage) {
                continue 'flows;
            }
        }
        // We didn't break, so this flow succeeded!
        completed = true;
    }

    if !completed {
        crate::uiaa::update_session(
            user_id,
            device_id,
            uiaa_info.session.as_ref().expect("session is always set"),
            Some(&uiaa_info),
        )
        .await?;
        return Ok((false, uiaa_info));
    }

    // UIAA was successful! Remove this session and return true
    crate::uiaa::update_session(
        user_id,
        device_id,
        uiaa_info.session.as_ref().expect("session is always set"),
        None,
    )
    .await?;
    Ok((true, uiaa_info))
}

/// Store the UIAA request body in the database for cross-instance access.
pub async fn set_uiaa_request(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
    request: CanonicalJsonValue,
) -> AppResult<()> {
    let request_body = serde_json::to_value(&request)?;
    diesel::update(
        user_uiaa_datas::table
            .filter(user_uiaa_datas::user_id.eq(user_id))
            .filter(user_uiaa_datas::device_id.eq(device_id))
            .filter(user_uiaa_datas::session.eq(session)),
    )
    .set(user_uiaa_datas::request_body.eq(Some(&request_body)))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Get the UIAA request body from the database.
pub async fn get_uiaa_request(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
) -> Option<CanonicalJsonValue> {
    let request_body = user_uiaa_datas::table
        .filter(user_uiaa_datas::user_id.eq(user_id))
        .filter(user_uiaa_datas::device_id.eq(device_id))
        .filter(user_uiaa_datas::session.eq(session))
        .select(user_uiaa_datas::request_body)
        .first::<Option<JsonValue>>(&mut connect().await.ok()?)
        .await
        .ok()
        .flatten()?;

    serde_json::from_value(request_body).ok()
}
