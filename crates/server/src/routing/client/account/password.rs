use diesel::prelude::*;
use palpo_core::client::account::ChangePasswordReqBody;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::uiaa::{AuthFlow, AuthType, UiaaInfo};
use crate::data::connect;
use crate::data::schema::*;
use crate::exts::*;
use crate::{AuthArgs, EmptyResult, SESSION_ID_LENGTH, empty_ok, hoops, utils};

pub fn authed_router() -> Router {
    Router::with_path("password")
        .hoop(hoops::limit_rate)
        .post(change_password)
}

/// #POST /_matrix/client/r0/account/password
/// Changes the password of this account.
///
/// - Requires UIAA to verify user password
/// - Changes the password of the sender user
/// - The password hash is calculated using argon2 with 32 character salt, the plain password is
/// not saved
///
/// If logout_devices is true it does the following for each device except the sender device:
/// - Invalidates access token
/// - Deletes device metadata (device id, device display name, last seen ip, last seen ts)
/// - Forgets to-device events
/// - Triggers device list updates
#[endpoint]
async fn change_password(
    req: &mut Request,
    _aa: AuthArgs,
    body: JsonBody<ChangePasswordReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    let mut uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec![AuthType::Password],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };
    let Some(auth) = &body.auth else {
        uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
        return Err(uiaa_info.into());
    };
    if crate::uiaa::try_auth(authed.user_id(), authed.device_id(), auth, &uiaa_info).is_err() {
        uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
        return Err(uiaa_info.into());
    }

    crate::user::set_password(authed.user_id(), &body.new_password)?;
    if let Some(access_token_id) = authed.access_token_id() {
        diesel::delete(
            user_pushers::table
                .filter(user_pushers::user_id.eq(authed.user_id()))
                .filter(user_pushers::access_token_id.ne(access_token_id)),
        )
        .execute(&mut connect()?)?;
    }
    if body.logout_devices {
        // Logout all devices except the current one
        diesel::delete(
            user_devices::table
                .filter(user_devices::user_id.eq(authed.user_id()))
                .filter(user_devices::device_id.ne(authed.device_id())),
        )
        .execute(&mut connect()?)?;
        diesel::delete(
            user_access_tokens::table
                .filter(user_access_tokens::user_id.eq(authed.user_id()))
                .filter(user_access_tokens::device_id.ne(authed.device_id())),
        )
        .execute(&mut connect()?)?;
        diesel::delete(
            user_refresh_tokens::table
                .filter(user_refresh_tokens::user_id.eq(authed.user_id()))
                .filter(user_refresh_tokens::device_id.ne(authed.device_id())),
        )
        .execute(&mut connect()?)?;
    }

    info!("User {} changed their password.", authed.user_id());
    // crate::admin::send_message(RoomMessageEventContent::notice_plain(format!("User {user} changed
    // their password.")));

    empty_ok()
}
