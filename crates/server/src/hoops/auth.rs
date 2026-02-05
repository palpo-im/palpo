use std::collections::BTreeMap;
use std::iter::FromIterator;

use diesel::prelude::*;
use salvo::http::headers::HeaderMapExt;
use salvo::http::headers::authorization::Authorization;
use salvo::prelude::*;

use crate::appservice::RegistrationInfo;
use crate::core::UnixMillis;
use crate::core::federation::authentication::XMatrix;
use crate::core::identifiers::*;
use crate::core::serde::CanonicalJsonValue;
use crate::core::signatures;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::{DbAccessToken, DbUser, DbUserDevice, NewDbUser, NewDbUserDevice};
use crate::exts::DepotExt;
use crate::server_key::{PubKeyMap, PubKeys};
use crate::{AppResult, AuthArgs, AuthedInfo, MatrixError, config};

#[handler]
pub async fn auth_by_access_token_or_signatures(
    aa: AuthArgs,
    req: &mut Request,
    depot: &mut Depot,
) -> AppResult<()> {
    if let Some(authorization) = &aa.authorization {
        if authorization.starts_with("Bearer ") {
            auth_by_access_token_inner(aa, depot).await
        } else {
            auth_by_signatures_inner(req, depot).await
        }
    } else {
        Err(MatrixError::missing_token("missing token").into())
    }
}

#[handler]
pub async fn auth_by_access_token(aa: AuthArgs, depot: &mut Depot) -> AppResult<()> {
    auth_by_access_token_inner(aa, depot).await
}
#[handler]
pub async fn auth_by_signatures(
    _aa: AuthArgs,
    req: &mut Request,
    depot: &mut Depot,
) -> AppResult<()> {
    auth_by_signatures_inner(req, depot).await
}

async fn auth_by_access_token_inner(aa: AuthArgs, depot: &mut Depot) -> AppResult<()> {
    let token = aa.require_access_token()?;
    let access_token = user_access_tokens::table
        .filter(user_access_tokens::token.eq(token))
        .first::<DbAccessToken>(&mut connect()?)
        .ok();
    if let Some(access_token) = access_token {
        let user = users::table
            .find(&access_token.user_id)
            .first::<DbUser>(&mut connect()?)
            .map_err(|_| MatrixError::unknown_token("User not found", true))?;
        let user_device = user_devices::table
            .filter(user_devices::device_id.eq(&access_token.device_id))
            .filter(user_devices::user_id.eq(&user.id))
            .first::<DbUserDevice>(&mut connect()?)
            .map_err(|_| MatrixError::unknown_token("User device not found", true))?;

        depot.inject(AuthedInfo {
            user,
            user_device,
            access_token_id: Some(access_token.id),
            appservice: None,
        });
        Ok(())
    } else {
        let appservices = crate::appservices();
        for appservice in appservices {
            if appservice.as_token == token {
                let appservice_info: RegistrationInfo = appservice.to_owned().try_into()?;

                // Check if the appservice is masquerading as another user
                let (user, user_device) = if let Some(ref user_id_str) = aa.user_id {
                    let user_id = UserId::parse(user_id_str)
                        .map_err(|_| MatrixError::invalid_param("Invalid user_id"))?;

                    // Verify the user is in the appservice's namespace
                    if !appservice_info.is_user_match(&user_id) {
                        return Err(MatrixError::forbidden(
                            "User is not in appservice's namespace",
                            None,
                        )
                        .into());
                    }

                    // Get or create the masqueraded user
                    let user = get_or_create_appservice_user(&user_id, &appservice.id)?;
                    let user_device = get_or_create_appservice_device(&user_id, aa.device_id.as_deref())?;
                    (user, user_device)
                } else {
                    // Use the appservice's main user
                    let user = users::table
                        .filter(users::appservice_id.eq(&appservice.id))
                        .first::<DbUser>(&mut connect()?)?;
                    let user_device = user_devices::table
                        .filter(user_devices::user_id.eq(&user.id))
                        .first::<DbUserDevice>(&mut connect()?)?;
                    (user, user_device)
                };

                depot.inject(AuthedInfo {
                    user,
                    user_device,
                    access_token_id: None,
                    appservice: Some(appservice_info),
                });
                return Ok(());
            }
        }
        Err(MatrixError::unknown_token("unknown access token", true).into())
    }
}

/// Get or create a user for an appservice
fn get_or_create_appservice_user(user_id: &UserId, appservice_id: &str) -> AppResult<DbUser> {
    // Try to get existing user
    if let Ok(user) = users::table.find(user_id).first::<DbUser>(&mut connect()?) {
        return Ok(user);
    }

    // Create new user for the appservice
    let new_user = NewDbUser {
        id: user_id.to_owned(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: user_id.server_name() == config::get().server_name,
        localpart: user_id.localpart().to_owned(),
        server_name: user_id.server_name().to_owned(),
        appservice_id: Some(appservice_id.to_owned()),
        created_at: UnixMillis::now(),
    };

    let user = diesel::insert_into(users::table)
        .values(&new_user)
        .on_conflict(users::id)
        .do_update()
        .set(&new_user)
        .get_result::<DbUser>(&mut connect()?)?;

    // Create a profile for the user
    let display_name = user_id.localpart().to_owned();
    if let Err(e) = crate::data::user::set_display_name(user_id, &display_name) {
        tracing::warn!("failed to set profile for appservice user (non-fatal): {}", e);
    }

    Ok(user)
}

/// Get or create a device for an appservice user
fn get_or_create_appservice_device(
    user_id: &UserId,
    device_id: Option<&str>,
) -> AppResult<DbUserDevice> {
    let device_id = device_id
        .map(|d| d.to_owned().into())
        .unwrap_or_else(|| DeviceId::new());

    // Try to get existing device
    if let Ok(device) = user_devices::table
        .filter(user_devices::user_id.eq(user_id))
        .filter(user_devices::device_id.eq(&device_id))
        .first::<DbUserDevice>(&mut connect()?)
    {
        return Ok(device);
    }

    // Create new device
    let new_device = NewDbUserDevice {
        user_id: user_id.to_owned(),
        device_id,
        display_name: None,
        user_agent: None,
        is_hidden: true,
        last_seen_ip: None,
        last_seen_at: None,
        created_at: UnixMillis::now(),
    };

    let device = diesel::insert_into(user_devices::table)
        .values(&new_device)
        .get_result::<DbUserDevice>(&mut connect()?)?;

    Ok(device)
}

async fn auth_by_signatures_inner(req: &mut Request, depot: &mut Depot) -> AppResult<()> {
    let Some(Authorization(x_matrix)) = req.headers().typed_get::<Authorization<XMatrix>>() else {
        warn!("missing or invalid Authorization header");
        return Err(MatrixError::forbidden("Missing or invalid authorization header", None).into());
    };

    let origin_signatures = BTreeMap::from_iter([(
        x_matrix.key.as_str().to_owned(),
        CanonicalJsonValue::String(x_matrix.sig.to_string()),
    )]);

    let origin = &x_matrix.origin;
    let signatures = BTreeMap::from_iter([(
        origin.as_str().to_owned(),
        CanonicalJsonValue::Object(origin_signatures),
    )]);

    let mut authorization = BTreeMap::from_iter([
        (
            "destination".to_owned(),
            CanonicalJsonValue::String(config::get().server_name.as_str().to_owned()),
        ),
        (
            "method".to_owned(),
            CanonicalJsonValue::String(req.method().to_string()),
        ),
        (
            "origin".to_owned(),
            CanonicalJsonValue::String(origin.as_str().to_owned()),
        ),
        (
            "uri".to_owned(),
            format!(
                "{}{}",
                req.uri().path(),
                req.uri()
                    .query()
                    .map(|q| format!("?{q}"))
                    .unwrap_or_default()
            )
            .into(),
        ),
        (
            "signatures".to_owned(),
            CanonicalJsonValue::Object(signatures),
        ),
    ]);

    let json_body = req
        .payload()
        .await
        .ok()
        .and_then(|payload| serde_json::from_slice::<CanonicalJsonValue>(payload).ok());

    if let Some(json_body) = &json_body {
        authorization.insert("content".to_owned(), json_body.clone());
    };

    let key = crate::server_key::get_verify_key(origin, &x_matrix.key).await?;

    let keys: PubKeys = [(x_matrix.key.to_string(), key.key)].into();
    let keys: PubKeyMap = [(origin.as_str().into(), keys)].into();
    if let Err(e) = signatures::verify_json(&keys, &authorization) {
        warn!(
            "Failed to verify json request from {}: {}\n{:?}",
            x_matrix.origin, e, authorization
        );

        if req.uri().to_string().contains('@') {
            warn!(
                "Request uri contained '@' character. Make sure your \
                                         reverse proxy gives Palpo the raw uri (apache: use \
                                         nocanon)"
            );
        }

        Err(MatrixError::forbidden("Failed to verify X-Matrix signatures.", None).into())
    } else {
        depot.set_origin(origin.to_owned());
        Ok(())
    }
}
