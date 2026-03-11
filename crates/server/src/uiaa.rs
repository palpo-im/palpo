use diesel::prelude::*;

use crate::core::client::uiaa::{
    AuthData, AuthError, AuthType, Password, UiaaInfo, UserIdentifier,
};
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonValue, JsonValue};
use crate::data::connect;
use crate::data::schema::*;
use crate::{AppResult, MatrixError, SESSION_ID_LENGTH, data, utils};

/// Creates a new Uiaa session. Make sure the session token is unique.
pub fn create_session(
    user_id: &UserId,
    device_id: &DeviceId,
    uiaa_info: &UiaaInfo,
    json_body: CanonicalJsonValue,
) -> AppResult<()> {
    let session = uiaa_info.session.as_ref().expect("session should be set");
    // First create/update the DB row with uiaa_info
    update_session(user_id, device_id, session, Some(uiaa_info))?;
    // Then store the request body (now the row exists)
    set_uiaa_request(user_id, device_id, session, json_body)?;
    Ok(())
}

pub fn update_session(
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
            .execute(&mut connect()?)?;
    } else {
        diesel::delete(
            user_uiaa_datas::table
                .filter(user_uiaa_datas::user_id.eq(user_id))
                .filter(user_uiaa_datas::device_id.eq(device_id))
                .filter(user_uiaa_datas::session.eq(session)),
        )
        .execute(&mut connect()?)?;
    };
    Ok(())
}
pub fn get_session(user_id: &UserId, device_id: &DeviceId, session: &str) -> AppResult<UiaaInfo> {
    let uiaa_info = user_uiaa_datas::table
        .filter(user_uiaa_datas::user_id.eq(user_id))
        .filter(user_uiaa_datas::device_id.eq(device_id))
        .filter(user_uiaa_datas::session.eq(session))
        .select(user_uiaa_datas::uiaa_info)
        .first::<JsonValue>(&mut connect()?)?;
    Ok(serde_json::from_value(uiaa_info)?)
}
pub fn try_auth(
    user_id: &UserId,
    device_id: &DeviceId,
    auth: &AuthData,
    uiaa_info: &UiaaInfo,
) -> AppResult<(bool, UiaaInfo)> {
    let mut uiaa_info = auth
        .session()
        .map(|session| get_session(user_id, device_id, session))
        .unwrap_or_else(|| Ok(uiaa_info.clone()))?;

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
                UserIdentifier::UserIdOrLocalpart(username) => username,
                _ => {
                    return Err(MatrixError::unauthorized("identifier type not recognized.").into());
                }
            };

            let auth_user_id = UserId::parse_with_server_name(username.clone(), &conf.server_name)
                .map_err(|_| MatrixError::unauthorized("User ID is invalid."))?;
            if user_id != auth_user_id {
                return Err(MatrixError::forbidden("User ID does not match.", None).into());
            }

            let Ok(user) = data::user::get_user(&auth_user_id) else {
                return Err(MatrixError::unauthorized("user not found.").into());
            };
            crate::user::verify_password(&user, password)?;
        }
        AuthData::RegistrationToken(t) => {
            if Some(t.token.trim()) == conf.registration_token.as_deref() {
                uiaa_info.completed.push(AuthType::RegistrationToken);
            } else {
                uiaa_info.auth_error =
                    Some(AuthError::forbidden("Invalid registration token."));
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
        )?;
        return Ok((false, uiaa_info));
    }

    // UIAA was successful! Remove this session and return true
    crate::uiaa::update_session(
        user_id,
        device_id,
        uiaa_info.session.as_ref().expect("session is always set"),
        None,
    )?;
    Ok((true, uiaa_info))
}

/// Store the UIAA request body in the database for cross-instance access.
pub fn set_uiaa_request(
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
    .execute(&mut connect()?)?;
    Ok(())
}

/// Get the UIAA request body from the database.
pub fn get_uiaa_request(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
) -> Option<CanonicalJsonValue> {
    let request_body = user_uiaa_datas::table
        .filter(user_uiaa_datas::user_id.eq(user_id))
        .filter(user_uiaa_datas::device_id.eq(device_id))
        .filter(user_uiaa_datas::session.eq(session))
        .select(user_uiaa_datas::request_body)
        .first::<Option<JsonValue>>(&mut connect().ok()?)
        .ok()
        .flatten()?;

    serde_json::from_value(request_body).ok()
}
